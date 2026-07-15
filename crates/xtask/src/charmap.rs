//! Gen 1 text charmap (pokered `constants/charmap.asm`).
//!
//! Used to sanity-decode the name strings in pokered's data files: each name
//! is encoded to Gen 1 bytes with the game's charmap (greedy longest match,
//! exactly like RGBDS does), then decoded back to a canonical UTF-8 string.
//! Any character outside the charmap fails loudly.

use std::path::Path;

pub struct Charmap {
    /// `(source text, gen1 byte)` pairs in file order.
    entries: Vec<(String, u8)>,
}

impl Charmap {
    pub fn load(path: &Path) -> Charmap {
        let text = std::fs::read_to_string(path)
            .unwrap_or_else(|e| panic!("read {}: {e}", path.display()));
        let mut entries = Vec::new();
        for (lineno, line) in text.lines().enumerate() {
            let trimmed = line.trim();
            let Some(rest) = trimmed.strip_prefix("charmap ") else {
                continue;
            };
            let ctx = || format!("{}:{}", path.display(), lineno + 1);
            let rest = rest.trim_start();
            let Some(rest) = rest.strip_prefix('"') else {
                panic!("{}: expected quoted string in charmap line", ctx());
            };
            let Some(end) = rest.find('"') else {
                panic!("{}: unterminated string in charmap line", ctx());
            };
            let key = rest[..end].to_string();
            let rest = rest[end + 1..].trim_start();
            let Some(rest) = rest.strip_prefix(',') else {
                panic!("{}: expected comma after charmap string", ctx());
            };
            let value = rest
                .split(';')
                .next()
                .unwrap_or("")
                .trim()
                .strip_prefix('$')
                .and_then(|v| u8::from_str_radix(v, 16).ok())
                .unwrap_or_else(|| panic!("{}: expected $XX byte value", ctx()));
            entries.push((key, value));
        }
        assert!(
            entries.len() > 100,
            "{}: suspiciously few charmap entries ({})",
            path.display(),
            entries.len()
        );
        Charmap { entries }
    }

    /// Encode a source string to Gen 1 bytes: greedy longest match, first
    /// entry wins on ties (mirrors RGBDS charmap semantics).
    fn encode(&self, s: &str, ctx: &str) -> Vec<u8> {
        let mut bytes = Vec::new();
        let mut rest = s;
        while !rest.is_empty() {
            let mut best: Option<(&str, u8)> = None;
            for (key, value) in &self.entries {
                if !key.is_empty()
                    && rest.starts_with(key.as_str())
                    && best.is_none_or(|(bk, _)| key.len() > bk.len())
                {
                    best = Some((key, *value));
                }
            }
            let Some((key, value)) = best else {
                panic!("{ctx}: no charmap entry matches remainder {rest:?} of {s:?}");
            };
            bytes.push(value);
            rest = &rest[key.len()..];
        }
        bytes
    }

    /// Canonical text for one Gen 1 byte: the first non-`<...>` charmap
    /// entry (shortest wins), e.g. `$80` -> "A", `$ef` -> "♂".
    fn decode_byte(&self, byte: u8, ctx: &str) -> &str {
        self.entries
            .iter()
            .filter(|(k, v)| *v == byte && !k.starts_with('<'))
            .map(|(k, _)| k.as_str())
            .min_by_key(|k| k.len())
            .unwrap_or_else(|| panic!("{ctx}: no printable charmap entry for byte ${byte:02X}"))
    }

    /// Round-trip a pokered source string through the Gen 1 charset,
    /// producing the canonical UTF-8 rendering (and failing loudly on any
    /// character the charset cannot represent).
    pub fn normalize(&self, s: &str, ctx: &str) -> String {
        self.encode(s, ctx)
            .iter()
            .map(|&b| self.decode_byte(b, ctx))
            .collect()
    }
}
