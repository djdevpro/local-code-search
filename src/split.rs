//! Analyseur « code » pour tantivy : chaque identifiant donne le mot entier puis ses
//! sous-mots à la même position (`get_connection_pool` → `get_connection_pool`, `get`,
//! `connection`, `pool`). Pas de racinisation, pas de mots vides.

use tantivy::tokenizer::{Token, TokenStream, Tokenizer};

pub const NAME: &str = "code";

/// Découpe un identifiant en sous-mots (snake_case, kebab, camelCase, chiffres).
pub fn subwords(ident: &str) -> Vec<String> {
    let mut parts = Vec::new();
    for piece in ident.split(['_', '-', '$']) {
        let chars: Vec<char> = piece.chars().collect();
        let mut current = String::new();
        for (i, &c) in chars.iter().enumerate() {
            let prev = if i > 0 { Some(chars[i - 1]) } else { None };
            let next = chars.get(i + 1).copied();
            let boundary = match prev {
                Some(p) => {
                    (p.is_lowercase() && c.is_uppercase())
                        || (p.is_alphabetic() && c.is_ascii_digit())
                        || (p.is_ascii_digit() && c.is_alphabetic())
                        || (p.is_uppercase() && c.is_uppercase() && next.is_some_and(char::is_lowercase))
                }
                None => false,
            };
            if boundary && !current.is_empty() {
                parts.push(std::mem::take(&mut current).to_lowercase());
            }
            current.push(c);
        }
        if !current.is_empty() {
            parts.push(current.to_lowercase());
        }
    }
    parts
}

/// Tous les jetons d'un texte : (mot entier en minuscules, sous-mots).
pub fn tokens(text: &str) -> Vec<(String, Vec<String>)> {
    words(text)
        .map(|(_, w)| {
            let subs = subwords(w);
            let whole = w.to_lowercase();
            let subs = if subs.len() > 1 { subs } else { Vec::new() };
            (whole, subs)
        })
        .collect()
}

fn words(text: &str) -> impl Iterator<Item = (usize, &str)> {
    let mut out = Vec::new();
    let mut start = None;
    for (i, c) in text.char_indices() {
        let word_char = c.is_alphanumeric() || c == '_';
        match (word_char, start) {
            (true, None) => start = Some(i),
            (false, Some(s)) => {
                out.push((s, &text[s..i]));
                start = None;
            }
            _ => {}
        }
    }
    if let Some(s) = start {
        out.push((s, &text[s..]));
    }
    out.into_iter().filter(|(_, w)| w.len() <= 64)
}

#[derive(Clone, Default)]
pub struct CodeTokenizer {
    buffer: Vec<Token>,
}

pub struct CodeTokenStream<'a> {
    tokens: &'a [Token],
    index: Option<usize>,
}

impl Tokenizer for CodeTokenizer {
    type TokenStream<'a> = CodeTokenStream<'a>;

    fn token_stream<'a>(&'a mut self, text: &'a str) -> CodeTokenStream<'a> {
        self.buffer.clear();
        for (position, (offset, word)) in words(text).enumerate() {
            let token = |t: String| Token {
                offset_from: offset,
                offset_to: offset + word.len(),
                position,
                text: t,
                position_length: 1,
            };
            self.buffer.push(token(word.to_lowercase()));
            let subs = subwords(word);
            if subs.len() > 1 {
                self.buffer.extend(subs.into_iter().map(token));
            }
        }
        CodeTokenStream { tokens: &self.buffer, index: None }
    }
}

impl TokenStream for CodeTokenStream<'_> {
    fn advance(&mut self) -> bool {
        let next = self.index.map_or(0, |i| i + 1);
        self.index = Some(next);
        next < self.tokens.len()
    }

    fn token(&self) -> &Token {
        &self.tokens[self.index.unwrap_or(0)]
    }

    fn token_mut(&mut self) -> &mut Token {
        unreachable!("CodeTokenStream est en lecture seule")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decoupe_les_identifiants() {
        assert_eq!(subwords("get_connection_pool"), ["get", "connection", "pool"]);
        assert_eq!(subwords("parseHTTPResponse2"), ["parse", "http", "response", "2"]);
        assert_eq!(subwords("x"), ["x"]);
    }

    #[test]
    fn sous_mots_a_la_meme_position() {
        let mut t = CodeTokenizer::default();
        let mut s = t.token_stream("loadConfig(path)");
        let mut seen = Vec::new();
        while s.advance() {
            seen.push((s.token().text.clone(), s.token().position));
        }
        let expected = [("loadconfig", 0), ("load", 0), ("config", 0), ("path", 1)];
        assert_eq!(seen, expected.map(|(t, p)| (t.to_string(), p)));
    }
}
