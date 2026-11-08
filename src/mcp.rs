//! Serveur MCP en stdio : 3 outils (`search`, `def`, `status`), paramètres de types simples,
//! descriptions courtes. Aucun port, aucune authentification, aucune consigne d'envoi.

use anyhow::Result;
use rmcp::handler::server::{router::tool::ToolRouter, wrapper::Parameters};
use rmcp::model::{MetaObject, ServerCapabilities, ServerConfig};
use rmcp::{ServerHandler, ServiceExt, tool, tool_handler, tool_router};
use schemars::JsonSchema;
use serde::Deserialize;
use std::sync::Arc;

/// Appel d'outil transmis au moteur.
pub enum Call {
    Search { query: String, limit: usize },
    Def { name: String },
    Status,
}

pub type Handler = Arc<dyn Fn(Call) -> String + Send + Sync>;

#[derive(Deserialize, JsonSchema)]
pub struct SearchArgs {
    /// What you are looking for, in the language of the code (usually English) and with its words:
    /// class and function names, technical terms. Translate the user's terms first if needed.
    pub query: String,
    /// Number of results (0 = configured default).
    #[serde(default)]
    pub limit: u32,
}

#[derive(Deserialize, JsonSchema)]
pub struct DefArgs {
    /// Symbol name, optionally qualified (`Parent::name`).
    pub name: String,
}

#[derive(Clone)]
pub struct Server {
    handler: Handler,
    tool_router: ToolRouter<Self>,
}

const SEARCH_DESC: &str = "Finds the code that answers a question (by meaning and by keywords). \
Write the query in the language of the code and with its words (class and function names, technical terms; \
usually English), even when the user writes in another language: translate first, keeping a domain word \
the code may use as is (\"mot de passe oublié\" → \"password reset request\"). \
Returns one result per file: file:lines, a focused snippet and the other matching passages of that file. \
Trust these results: open only the files you will change or must understand, and only the lines you need \
(in Claude Code, Read with offset and limit rather than a shell command); do not grep again to confirm a file \
already found. \
For several aspects of a request, run several searches in the same turn. \
Use grep only for an exhaustive search (every call of X) or an exact string.";
const DEF_DESC: &str = "Returns the exact definition of a symbol (function, class, struct…) with file:lines.";
const STATUS_DESC: &str = "Index status: files, chunks, vector coverage, hardware backend.";

/// `_meta` des outils principaux : Claude Code les charge d'office au lieu de les différer derrière
/// sa recherche d'outils (mesuré : outils différés → l'agent ignore lcs). Le démarrage n'attend pas.
fn always_load() -> MetaObject {
    MetaObject(serde_json::Map::from_iter([("anthropic/alwaysLoad".to_string(), serde_json::Value::Bool(true))]))
}

impl Server {
    async fn call(&self, call: Call) -> String {
        let handler = self.handler.clone();
        tokio::task::spawn_blocking(move || handler(call)).await.unwrap_or_else(|e| format!("erreur interne : {e}"))
    }
}

#[tool_router]
impl Server {
    #[tool(description = SEARCH_DESC, meta = always_load())]
    async fn search(&self, Parameters(args): Parameters<SearchArgs>) -> String {
        self.call(Call::Search { query: args.query, limit: args.limit as usize }).await
    }

    #[tool(description = DEF_DESC, meta = always_load())]
    async fn def(&self, Parameters(args): Parameters<DefArgs>) -> String {
        self.call(Call::Def { name: args.name }).await
    }

    #[tool(description = STATUS_DESC)]
    async fn status(&self) -> String {
        self.call(Call::Status).await
    }
}

#[tool_handler(router = self.tool_router)]
impl ServerHandler for Server {
    fn get_info(&self) -> ServerConfig {
        let mut info = ServerConfig::default();
        info.capabilities = ServerCapabilities::builder().enable_tools().build();
        info.server_info.name = "local-code-search".into();
        info.server_info.version = env!("CARGO_PKG_VERSION").into();
        info.instructions = Some(
            "Local code search. Start with `search` to locate the code a request is about, `def` for a definition. \
             Write `search` queries in the language of the code (usually English) and with its words, even when \
             the user writes in another language: translate the terms of the request first. \
             Trust the results: read only the files you will change or must understand, do not grep again to \
             confirm a file `search` already found, and run several searches in the same turn when a request has \
             several aspects. Keep grep for exhaustive or literal searches (every call of X, an exact string)."
                .into(),
        );
        info
    }
}

/// Bloque jusqu'à la fin de la session MCP (stdin fermé).
pub fn serve(handler: Handler) -> Result<()> {
    let runtime = tokio::runtime::Builder::new_multi_thread().enable_all().build()?;
    runtime.block_on(async {
        let server = Server { handler, tool_router: Server::tool_router() };
        server.serve(rmcp::transport::stdio()).await?.waiting().await?;
        Ok(())
    })
}
