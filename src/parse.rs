//! tree-sitter → symboles (fonctions, classes, structs, impl…). Les lignes hors symbole et les
//! fichiers sans grammaire deviennent des fenêtres de lignes : jamais zéro enregistrement.

use crate::types::Unit;
use tree_sitter::{Language, Node, Parser};

const WINDOW_LINES: usize = 40;

/// Langage d'un fichier d'après son extension (parmi ceux activés dans la config).
pub fn language_of(path: &str, enabled: &[String]) -> Option<&'static str> {
    let ext = path.rsplit('.').next()?.to_ascii_lowercase();
    let lang = match ext.as_str() {
        "py" | "pyi" => "python",
        "ts" | "mts" | "cts" => "typescript",
        "tsx" => "tsx",
        "js" | "jsx" | "mjs" | "cjs" => "javascript",
        "rs" => "rust",
        "go" => "go",
        "php" => "php",
        _ => return None,
    };
    let family = if lang == "tsx" { "typescript" } else { lang };
    enabled.iter().any(|l| l == family).then_some(lang)
}

pub fn grammar(lang: &str) -> Option<Language> {
    Some(match lang {
        "python" => tree_sitter_python::LANGUAGE.into(),
        "typescript" => tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into(),
        "tsx" => tree_sitter_typescript::LANGUAGE_TSX.into(),
        "javascript" => tree_sitter_javascript::LANGUAGE.into(),
        "rust" => tree_sitter_rust::LANGUAGE.into(),
        "go" => tree_sitter_go::LANGUAGE.into(),
        "php" => tree_sitter_php::LANGUAGE_PHP.into(),
        _ => return None,
    })
}

/// (type de nœud, libellé, conteneur ?)
fn unit_kind(lang: &str, kind: &str) -> Option<(&'static str, bool)> {
    Some(match (lang, kind) {
        ("rust", "function_item" | "function_signature_item") => ("function", false),
        ("rust", "struct_item") => ("struct", false),
        ("rust", "enum_item") => ("enum", false),
        ("rust", "union_item") => ("union", false),
        ("rust", "trait_item") => ("trait", true),
        ("rust", "impl_item") => ("impl", true),
        ("rust", "mod_item") => ("module", true),
        ("rust", "const_item" | "static_item") => ("const", false),
        ("rust", "type_item") => ("type", false),
        ("rust", "macro_definition") => ("macro", false),
        ("python", "function_definition") => ("function", false),
        ("python", "class_definition") => ("class", true),
        ("go", "function_declaration") => ("function", false),
        ("go", "method_declaration") => ("method", false),
        ("go", "type_declaration") => ("type", false),
        ("php", "function_definition") => ("function", false),
        ("php", "method_declaration") => ("method", false),
        ("php", "class_declaration") => ("class", true),
        ("php", "trait_declaration") => ("trait", true),
        ("php", "interface_declaration") => ("interface", true),
        ("php", "enum_declaration") => ("enum", true),
        (_, "function_declaration" | "generator_function_declaration") => ("function", false),
        (_, "class_declaration" | "abstract_class_declaration") => ("class", true),
        (_, "method_definition") => ("method", false),
        (_, "interface_declaration") => ("interface", false),
        (_, "type_alias_declaration") => ("type", false),
        (_, "enum_declaration") => ("enum", false),
        (_, "variable_declarator") => ("function", false),
        _ => return None,
    })
}

pub fn units(lang: &str, source: &str) -> Vec<Unit> {
    let Some(language) = grammar(lang) else { return windows(source, 1, &lines(source)) };
    let mut parser = Parser::new();
    let tree = parser.set_language(&language).ok().and_then(|_| parser.parse(source, None));
    let Some(tree) = tree else { return windows(source, 1, &lines(source)) };

    let src = Source { text: source, lines: lines(source), lang };
    let mut out = Vec::new();
    visit(&src, tree.root_node(), None, &mut out);

    let mut covered = vec![false; src.lines.len() + 2];
    for u in out.iter().filter(|u| u.parent.is_none()) {
        covered[u.start..=u.end.min(src.lines.len())].iter_mut().for_each(|c| *c = true);
    }
    out.extend(uncovered_windows(&src, &covered));
    out.sort_by_key(|u| (u.start, u.end));
    out
}

struct Source<'a> {
    text: &'a str,
    lines: Vec<&'a str>,
    lang: &'a str,
}

fn lines(text: &str) -> Vec<&str> {
    text.lines().collect()
}

/// Parcourt les enfants de `node` ; renvoie les plages (lignes) des symboles directs trouvés.
fn visit(src: &Source, node: Node, parent: Option<&str>, out: &mut Vec<Unit>) -> Vec<(usize, usize)> {
    let mut ranges = Vec::new();
    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        let (def, outer) = match child.kind() {
            "decorated_definition" => match child.child_by_field_name("definition") {
                Some(d) => (d, child),
                None => continue,
            },
            _ => (child, child),
        };
        let Some((label, container)) = unit_kind(src.lang, def.kind()) else {
            ranges.extend(visit(src, child, parent, out));
            continue;
        };
        if def.kind() == "variable_declarator" && !is_function_value(def) {
            continue;
        }
        let name = name_of(src, def);
        let qualified = match parent {
            Some(p) => format!("{p}::{name}"),
            None => name.clone(),
        };
        let (start, end) = (outer.start_position().row + 1, outer.end_position().row + 1);
        let children = if container { visit(src, def, Some(&qualified), out) } else { Vec::new() };
        out.push(Unit {
            kind: label.to_string(),
            name,
            parent: parent.map(String::from),
            start,
            end,
            signature: signature(src, def),
            doc: doc_comments(src, outer),
            body: collapsed(src, start, end, &children),
        });
        ranges.push((start, end));
    }
    ranges
}

fn is_function_value(declarator: Node) -> bool {
    declarator
        .child_by_field_name("value")
        .is_some_and(|v| matches!(v.kind(), "arrow_function" | "function_expression" | "function"))
}

fn text<'a>(src: &Source<'a>, node: Node) -> &'a str {
    &src.text[node.byte_range()]
}

fn name_of(src: &Source, def: Node) -> String {
    if def.kind() == "impl_item" {
        let ty = def.child_by_field_name("type").map(|n| text(src, n)).unwrap_or("?");
        return match def.child_by_field_name("trait") {
            Some(t) => format!("{} for {ty}", text(src, t)),
            None => ty.to_string(),
        };
    }
    if def.kind() == "type_declaration" {
        let mut cursor = def.walk();
        let spec = def.named_children(&mut cursor).find(|c| c.kind() == "type_spec");
        if let Some(n) = spec.and_then(|s| s.child_by_field_name("name")) {
            return text(src, n).to_string();
        }
    }
    def.child_by_field_name("name").map_or_else(|| "?".into(), |n| text(src, n).to_string())
}

fn signature(src: &Source, def: Node) -> String {
    let line = src.lines.get(def.start_position().row).copied().unwrap_or("");
    let sig = line.trim().trim_end_matches('{').trim_end();
    sig.chars().take(200).collect()
}

fn doc_comments(src: &Source, node: Node) -> String {
    let mut docs = Vec::new();
    let mut next_start = node.start_position().row;
    let mut prev = node.prev_named_sibling();
    while let Some(p) = prev {
        if !p.kind().contains("comment") || p.end_position().row + 1 < next_start {
            break;
        }
        docs.push(text(src, p).trim_end());
        next_start = p.start_position().row;
        prev = p.prev_named_sibling();
    }
    docs.reverse();
    docs.join("\n")
}

/// Corps d'un symbole ; dans un conteneur, le corps des symboles enfants est réduit à leur
/// signature (ils ont leur propre morceau).
fn collapsed(src: &Source, start: usize, end: usize, children: &[(usize, usize)]) -> String {
    let mut out = Vec::new();
    let mut line = start;
    while line <= end.min(src.lines.len()) {
        if let Some(&(s, e)) = children.iter().find(|(s, _)| *s == line) {
            out.push(src.lines[s - 1].to_string());
            if e > s {
                out.push("    …".to_string());
            }
            line = e + 1;
        } else {
            out.push(src.lines[line - 1].to_string());
            line += 1;
        }
    }
    out.join("\n")
}

#[allow(clippy::needless_range_loop)]
fn uncovered_windows(src: &Source, covered: &[bool]) -> Vec<Unit> {
    let mut out = Vec::new();
    let mut run_start = None;
    for line in 1..=src.lines.len() + 1 {
        let free = line <= src.lines.len() && !covered[line];
        match (free, run_start) {
            (true, None) => run_start = Some(line),
            (false, Some(s)) => {
                out.extend(windows(src.text, s, &src.lines[s - 1..line - 1]));
                run_start = None;
            }
            _ => {}
        }
    }
    out
}

/// Fenêtres d'environ 40 lignes ; les fenêtres vides sont ignorées.
fn windows(_text: &str, first_line: usize, lines: &[&str]) -> Vec<Unit> {
    lines
        .chunks(WINDOW_LINES)
        .enumerate()
        .filter(|(_, w)| w.iter().any(|l| !l.trim().is_empty()))
        .map(|(i, w)| {
            let start = first_line + i * WINDOW_LINES;
            Unit {
                kind: "lines".into(),
                name: String::new(),
                parent: None,
                start,
                end: start + w.len() - 1,
                signature: w.iter().find(|l| !l.trim().is_empty()).map_or("", |l| l.trim()).chars().take(200).collect(),
                doc: String::new(),
                body: w.join("\n"),
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extrait_les_symboles_rust() {
        let src = "use std::io;\n\n/// Doc A\nfn a() {\n    1;\n}\n\nimpl Foo {\n    fn b(&self) {}\n}\n";
        let u = units("rust", src);
        let names: Vec<_> = u.iter().map(|u| (u.kind.as_str(), u.name.as_str())).collect();
        assert!(names.contains(&("function", "a")));
        assert!(names.contains(&("impl", "Foo")));
        assert!(names.contains(&("function", "b")));
        assert!(names.contains(&("lines", "")), "la ligne `use` n'est pas perdue");
        let a = u.iter().find(|u| u.name == "a").unwrap();
        assert_eq!((a.start, a.end, a.doc.as_str()), (4, 6, "/// Doc A"));
        assert_eq!(u.iter().find(|u| u.name == "b").unwrap().parent.as_deref(), Some("Foo"));
    }

    #[test]
    fn extrait_les_symboles_python_et_ts() {
        let py = units("python", "@dec\ndef f(x):\n    return x\n\nclass C:\n    def m(self):\n        pass\n");
        assert!(py.iter().any(|u| u.name == "f" && u.start == 1));
        assert!(py.iter().any(|u| u.name == "m" && u.parent.as_deref() == Some("C")));
        let ts = units("typescript", "export const add = (a: number) => a + 1;\nclass K { run() {} }\n");
        assert!(ts.iter().any(|u| u.name == "add" && u.kind == "function"));
        assert!(ts.iter().any(|u| u.name == "run" && u.kind == "method"));
    }

    #[test]
    fn extrait_les_symboles_php() {
        let src = r"<?php
namespace App\Http;

class UserController extends Controller
{
    public function store(Request $r)
    {
        return 1;
    }
}
";
        let u = units("php", src);
        assert!(u.iter().any(|u| u.kind == "class" && u.name == "UserController"), "{u:?}");
        assert!(u.iter().any(|u| u.kind == "method" && u.name == "store" && u.parent.as_deref() == Some("UserController")));
    }

    #[test]
    fn fichier_sans_grammaire_en_fenetres() {
        let text = (1..=90).map(|i| format!("ligne {i}")).collect::<Vec<_>>().join("\n");
        let u = units("markdown", &text);
        assert_eq!(u.iter().map(|u| (u.start, u.end)).collect::<Vec<_>>(), [(1, 40), (41, 80), (81, 90)]);
    }
}
