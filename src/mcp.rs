//! Serveur MCP en stdio : 3 outils (`search`, `def`, `status`), paramètres de types simples,
//! descriptions courtes. Aucun port, aucune authentification, aucune consigne d'envoi.

use anyhow::Result;
use rmcp::handler::server::{router::tool::ToolRouter, wrapper::Parameters};
use rmcp::model::{ServerCapabilities, ServerConfig};
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
    /// Ce que tu cherches, en langage naturel ou en termes de code.
    pub query: String,
    /// Nombre de résultats (0 = valeur de la configuration).
    #[serde(default)]
    pub limit: u32,
}

#[derive(Deserialize, JsonSchema)]
pub struct DefArgs {
    /// Nom du symbole, éventuellement qualifié (`Parent::nom`).
    pub name: String,
}

#[derive(Clone)]
pub struct Server {
    handler: Handler,
    tool_router: ToolRouter<Self>,
}

const SEARCH_DESC: &str = "Trouve les passages de code qui répondent à une question (par le sens et par les mots-clés). \
Renvoie fichier:lignes et un extrait ciblé. Pour voir plus d'un passage, utilise Read avec offset et limit \
(pas de commande shell). Pour une recherche exhaustive ou une chaîne exacte, utilise plutôt grep.";
const DEF_DESC: &str = "Renvoie la définition exacte d'un symbole (fonction, classe, struct…) avec fichier:lignes.";
const STATUS_DESC: &str = "État de l'index : fichiers, morceaux, couverture des vecteurs, backend matériel.";

impl Server {
    async fn call(&self, call: Call) -> String {
        let handler = self.handler.clone();
        tokio::task::spawn_blocking(move || handler(call)).await.unwrap_or_else(|e| format!("erreur interne : {e}"))
    }
}

#[tool_router]
impl Server {
    #[tool(description = SEARCH_DESC)]
    async fn search(&self, Parameters(args): Parameters<SearchArgs>) -> String {
        self.call(Call::Search { query: args.query, limit: args.limit as usize }).await
    }

    #[tool(description = DEF_DESC)]
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
            "Recherche de code locale. `search` pour localiser ou comprendre du code, `def` pour une définition, \
             grep pour les recherches exhaustives ou littérales."
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
