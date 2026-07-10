// SPDX-License-Identifier: Apache-2.0
//! SQL compatibility rewrites applied at QuackGIS trust boundaries.

use std::sync::LazyLock;

use datafusion::sql::sqlparser::dialect::PostgreSqlDialect;
use datafusion::sql::sqlparser::tokenizer::{Token, Tokenizer};

/// Lower the supported `AS OF SNAPSHOT` and `AS OF TIMESTAMP` grammar fragments
/// to named table selectors before the PostgreSQL dialect parser runs.
pub(super) fn rewrite_as_of_snapshot_selectors(sql: &str) -> Option<String> {
    let dialect = PostgreSqlDialect {};
    let tokens = Tokenizer::new(&dialect, sql)
        .with_unescape(false)
        .tokenize()
        .ok()?;
    let code_tokens = tokens
        .iter()
        .enumerate()
        .filter_map(|(idx, token)| (!matches!(token, Token::Whitespace(_))).then_some(idx))
        .collect::<Vec<_>>();
    let mut replacements = Vec::new();
    let mut pos = 0;

    while pos + 3 < code_tokens.len() {
        let as_pos = pos;
        let as_idx = code_tokens[as_pos];
        if !token_word_eq(&tokens[as_idx], "as")
            || !token_word_eq(&tokens[code_tokens[as_pos + 1]], "of")
        {
            pos += 1;
            continue;
        }

        let selector_kind = if token_word_eq(&tokens[code_tokens[as_pos + 2]], "snapshot") {
            "snapshot"
        } else if token_word_eq(&tokens[code_tokens[as_pos + 2]], "timestamp") {
            "snapshot_at"
        } else {
            pos += 1;
            continue;
        };

        let Some((table_start_pos, table_end_pos)) =
            preceding_table_name(&tokens, &code_tokens, as_pos)
        else {
            pos += 1;
            continue;
        };
        let Some((snapshot_start_pos, snapshot_end_pos)) =
            snapshot_selector_literal(&tokens, &code_tokens, as_pos + 3, selector_kind)
        else {
            pos += 1;
            continue;
        };

        if !snapshot_literal_has_boundary(&tokens, &code_tokens, snapshot_end_pos) {
            pos += 1;
            continue;
        }

        replacements.push((
            code_tokens[table_start_pos],
            code_tokens[table_end_pos],
            code_tokens[snapshot_start_pos],
            code_tokens[snapshot_end_pos],
            selector_kind,
        ));
        pos = snapshot_end_pos + 1;
    }

    if replacements.is_empty() {
        return None;
    }

    let mut out = String::with_capacity(sql.len());
    let mut cursor = 0;
    for (table_start, table_end, snapshot_start, snapshot_end, selector_kind) in replacements {
        append_sql_tokens(&mut out, &tokens[cursor..table_start]);
        append_sql_tokens(&mut out, &tokens[table_start..=table_end]);
        out.push('(');
        out.push_str(selector_kind);
        out.push_str(" => ");
        append_sql_tokens(&mut out, &tokens[snapshot_start..=snapshot_end]);
        out.push(')');
        cursor = snapshot_end + 1;
    }
    append_sql_tokens(&mut out, &tokens[cursor..]);
    Some(out)
}

fn snapshot_selector_literal(
    tokens: &[Token],
    code_tokens: &[usize],
    start_pos: usize,
    selector_kind: &str,
) -> Option<(usize, usize)> {
    if selector_kind == "snapshot" {
        snapshot_integer_literal(tokens, code_tokens, start_pos)
    } else {
        let token = tokens.get(*code_tokens.get(start_pos)?)?;
        matches!(token, Token::SingleQuotedString(_)).then_some((start_pos, start_pos))
    }
}

fn preceding_table_name(
    tokens: &[Token],
    code_tokens: &[usize],
    as_pos: usize,
) -> Option<(usize, usize)> {
    if as_pos < 2 {
        return None;
    }
    let mut table_start_pos = as_pos - 1;
    if !token_is_identifier(&tokens[code_tokens[table_start_pos]]) {
        return None;
    }

    while table_start_pos >= 2
        && matches!(tokens[code_tokens[table_start_pos - 1]], Token::Period)
        && token_is_identifier(&tokens[code_tokens[table_start_pos - 2]])
    {
        table_start_pos -= 2;
    }

    if table_start_pos == 0 {
        return None;
    }
    let before_table = &tokens[code_tokens[table_start_pos - 1]];
    if token_word_eq(before_table, "from") || token_word_eq(before_table, "join") {
        Some((table_start_pos, as_pos - 1))
    } else {
        None
    }
}

fn snapshot_integer_literal(
    tokens: &[Token],
    code_tokens: &[usize],
    start_pos: usize,
) -> Option<(usize, usize)> {
    let first = tokens.get(*code_tokens.get(start_pos)?)?;
    if matches!(first, Token::Plus) {
        let number_pos = start_pos + 1;
        let number = tokens.get(*code_tokens.get(number_pos)?)?;
        if token_is_unsigned_integer(number) {
            return Some((start_pos, number_pos));
        }
        return None;
    }
    if token_is_unsigned_integer(first) {
        Some((start_pos, start_pos))
    } else {
        None
    }
}

fn snapshot_literal_has_boundary(
    tokens: &[Token],
    code_tokens: &[usize],
    snapshot_end_pos: usize,
) -> bool {
    let snapshot_idx = code_tokens[snapshot_end_pos];
    let Some(next_idx) = code_tokens.get(snapshot_end_pos + 1) else {
        return true;
    };
    let next = &tokens[*next_idx];
    !(*next_idx == snapshot_idx + 1
        && (token_is_identifier(next) || matches!(next, Token::Number(_, _))))
}

fn token_is_unsigned_integer(token: &Token) -> bool {
    matches!(token, Token::Number(raw, false) if raw.bytes().all(|byte| byte.is_ascii_digit()))
}

fn token_word_eq(token: &Token, expected: &str) -> bool {
    matches!(token, Token::Word(word) if word.quote_style.is_none() && word.value.eq_ignore_ascii_case(expected))
}

fn token_is_identifier(token: &Token) -> bool {
    matches!(token, Token::Word(_))
}

fn append_sql_tokens(out: &mut String, tokens: &[Token]) {
    for token in tokens {
        out.push_str(&token.to_string());
    }
}

pub(super) fn rewrite_pg_escape_bytea_literals(sql: &str) -> String {
    let bytes = sql.as_bytes();
    let mut out = String::with_capacity(sql.len());
    let mut i = 0;
    while i < bytes.len() {
        if (bytes[i] == b'E' || bytes[i] == b'e') && bytes.get(i + 1) == Some(&b'\'') {
            let body_start = i + 2;
            if let Some(literal_end) = quoted_literal_end(bytes, body_start) {
                let literal = &sql[i..=literal_end];
                let body = &sql[body_start..literal_end];
                if let Some(decoded) = decode_pg_escape_bytea_body(body) {
                    out.push_str("X'");
                    out.push_str(&hex_encode(&decoded));
                    out.push('\'');
                } else if let Some(decoded_text) = decode_pg_escape_text_body(body) {
                    out.push('\'');
                    out.push_str(&decoded_text.replace('\'', "''"));
                    out.push('\'');
                } else {
                    out.push_str(literal);
                }
                i = literal_end + 1;
                continue;
            }
        }
        let start = i;
        i += 1;
        while i < bytes.len()
            && !((bytes[i] == b'E' || bytes[i] == b'e') && bytes.get(i + 1) == Some(&b'\''))
        {
            i += 1;
        }
        out.push_str(&sql[start..i]);
    }
    out
}

pub(super) fn rewrite_st_geomfromwkb_zero_srid_literals(sql: &str) -> String {
    static ST_GEOMFROMWKB_RE: LazyLock<regex::Regex> = LazyLock::new(|| {
        regex::Regex::new(
            r"(?i)\bst_geomfromwkb\s*\(\s*(?P<wkb>X'(?P<hex>[0-9a-f]*)'|NULL)\s*(?:::bytea)?\s*,\s*(?P<srid>\d+)\s*\)",
        )
        .expect("valid ST_GeomFromWKB rewrite regex")
    });

    ST_GEOMFROMWKB_RE
        .replace_all(sql, |captures: &regex::Captures<'_>| {
            let wkb = captures.name("wkb").map(|m| m.as_str()).unwrap_or("NULL");
            let srid = captures
                .name("srid")
                .and_then(|m| m.as_str().parse::<u32>().ok())
                .unwrap_or(0);
            if srid == 0 || wkb.eq_ignore_ascii_case("NULL") {
                return wkb.to_string();
            }
            captures
                .name("hex")
                .and_then(|m| hex_decode(m.as_str()))
                .and_then(|bytes| tag_wkb_srid(&bytes, srid))
                .map(|bytes| format!("X'{}'", hex_encode(&bytes)))
                .unwrap_or_else(|| {
                    captures
                        .get(0)
                        .map(|m| m.as_str())
                        .unwrap_or(wkb)
                        .to_string()
                })
        })
        .into_owned()
}

pub(super) fn rewrite_mojibake_string_literals(sql: &str) -> String {
    let bytes = sql.as_bytes();
    let mut out = String::with_capacity(sql.len());
    let mut i = 0;
    while i < bytes.len() {
        let (literal_start, body_start, prefix_is_hex) = if bytes[i] == b'\'' {
            (i, i + 1, false)
        } else if matches!(bytes[i], b'E' | b'e' | b'X' | b'x') && bytes.get(i + 1) == Some(&b'\'')
        {
            (i, i + 2, matches!(bytes[i], b'X' | b'x'))
        } else {
            let start = i;
            i += 1;
            while i < bytes.len()
                && bytes[i] != b'\''
                && !(matches!(bytes[i], b'E' | b'e' | b'X' | b'x')
                    && bytes.get(i + 1) == Some(&b'\''))
            {
                i += 1;
            }
            out.push_str(&sql[start..i]);
            continue;
        };

        if let Some(literal_end) = quoted_literal_end(bytes, body_start) {
            if prefix_is_hex {
                out.push_str(&sql[literal_start..=literal_end]);
            } else {
                let body = &sql[body_start..literal_end];
                let unescaped = body.replace("''", "'");
                if let Some(repaired) = repair_latin1_decoded_utf8_mojibake(&unescaped) {
                    out.push('\'');
                    out.push_str(&repaired.replace('\'', "''"));
                    out.push('\'');
                } else {
                    out.push_str(&sql[literal_start..=literal_end]);
                }
            }
            i = literal_end + 1;
        } else {
            out.push_str(&sql[literal_start..]);
            break;
        }
    }
    out
}

fn quoted_literal_end(bytes: &[u8], body_start: usize) -> Option<usize> {
    let mut i = body_start;
    while i < bytes.len() {
        if bytes[i] == b'\'' {
            if bytes.get(i + 1) == Some(&b'\'') {
                i += 2;
            } else {
                return Some(i);
            }
        } else {
            i += 1;
        }
    }
    None
}

pub(super) fn repair_latin1_decoded_utf8_mojibake(value: &str) -> Option<String> {
    if !looks_like_latin1_decoded_utf8(value) {
        return None;
    }
    let mut current = value.to_string();
    for _ in 0..3 {
        let bytes = latin1_bytes(&current)?;
        let repaired = String::from_utf8(bytes).ok()?;
        if repaired == current {
            break;
        }
        current = repaired;
        if !looks_like_latin1_decoded_utf8(&current) {
            return Some(current);
        }
    }
    (current != value).then_some(current)
}

fn looks_like_latin1_decoded_utf8(value: &str) -> bool {
    value
        .chars()
        .any(|ch| matches!(ch, 'Ã' | 'Â') || ('\u{80}'..='\u{9f}').contains(&ch))
}

fn latin1_bytes(value: &str) -> Option<Vec<u8>> {
    value
        .chars()
        .map(|ch| (u32::from(ch) <= 0xff).then_some(ch as u8))
        .collect()
}

pub(super) fn decode_pg_escape_bytea_body(body: &str) -> Option<Vec<u8>> {
    let out = decode_pg_escape_octal_body(body)?;
    looks_like_wkb(&out).then_some(out)
}

fn decode_pg_escape_text_body(body: &str) -> Option<String> {
    let out = decode_pg_escape_octal_body(body)?;
    if looks_like_wkb(&out) {
        return None;
    }
    String::from_utf8(out).ok()
}

fn decode_pg_escape_octal_body(body: &str) -> Option<Vec<u8>> {
    let bytes = body.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut has_octal = false;
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'\\' {
            let octal_start = if bytes.get(i + 1) == Some(&b'\\') {
                i + 2
            } else {
                i + 1
            };
            if octal_start + 3 <= bytes.len()
                && bytes[octal_start..octal_start + 3]
                    .iter()
                    .all(|b| (b'0'..=b'7').contains(b))
            {
                let value = (bytes[octal_start] - b'0') * 64
                    + (bytes[octal_start + 1] - b'0') * 8
                    + (bytes[octal_start + 2] - b'0');
                out.push(value);
                has_octal = true;
                i = octal_start + 3;
                continue;
            }
            return None;
        }
        out.push(bytes[i]);
        i += 1;
    }
    has_octal.then_some(out)
}

fn looks_like_wkb(bytes: &[u8]) -> bool {
    if bytes.len() < 5 || !matches!(bytes[0], 0 | 1) {
        return false;
    }
    let type_bytes = [bytes[1], bytes[2], bytes[3], bytes[4]];
    let raw_type = if bytes[0] == 0 {
        u32::from_be_bytes(type_bytes)
    } else {
        u32::from_le_bytes(type_bytes)
    };
    let type_id = raw_type & 0x0fff;
    let base_type = if type_id >= 1000 {
        type_id % 1000
    } else {
        type_id
    };
    (1..=7).contains(&base_type)
}

pub(super) fn hex_encode(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        out.push(HEX[(byte >> 4) as usize] as char);
        out.push(HEX[(byte & 0x0f) as usize] as char);
    }
    out
}

fn hex_decode(hex: &str) -> Option<Vec<u8>> {
    if !hex.len().is_multiple_of(2) {
        return None;
    }
    (0..hex.len())
        .step_by(2)
        .map(|idx| u8::from_str_radix(&hex[idx..idx + 2], 16).ok())
        .collect()
}

fn tag_wkb_srid(wkb: &[u8], srid: u32) -> Option<Vec<u8>> {
    if wkb.len() < 5 || !matches!(wkb[0], 0 | 1) {
        return None;
    }
    let byte_order = wkb[0];
    let type_bytes = [wkb[1], wkb[2], wkb[3], wkb[4]];
    let type_id = if byte_order == 0 {
        u32::from_be_bytes(type_bytes)
    } else {
        u32::from_le_bytes(type_bytes)
    } | 0x2000_0000;

    let mut out = Vec::with_capacity(wkb.len() + 4);
    out.push(byte_order);
    if byte_order == 0 {
        out.extend_from_slice(&type_id.to_be_bytes());
        out.extend_from_slice(&srid.to_be_bytes());
    } else {
        out.extend_from_slice(&type_id.to_le_bytes());
        out.extend_from_slice(&srid.to_le_bytes());
    }
    out.extend_from_slice(&wkb[5..]);
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rewrites_as_of_snapshot_table_selector() {
        let sql = "SELECT id FROM public.assets AS OF SNAPSHOT 42 WHERE id = 7";

        assert_eq!(
            rewrite_as_of_snapshot_selectors(sql).as_deref(),
            Some("SELECT id FROM public.assets(snapshot => 42) WHERE id = 7")
        );
    }

    #[test]
    fn rewrites_quoted_as_of_snapshot_table_selector_with_alias() {
        let sql = "SELECT a.id FROM \"public\".\"asset table\" AS OF SNAPSHOT +42 AS a";

        assert_eq!(
            rewrite_as_of_snapshot_selectors(sql).as_deref(),
            Some("SELECT a.id FROM \"public\".\"asset table\"(snapshot => +42) AS a")
        );
    }

    #[test]
    fn ignores_as_of_snapshot_outside_from_or_join() {
        let sql = "SELECT value AS OF SNAPSHOT 42 FROM public.assets";

        assert_eq!(rewrite_as_of_snapshot_selectors(sql), None);
    }

    #[test]
    fn ignores_as_of_snapshot_inside_strings_and_comments() {
        let sql = "SELECT 'FROM public.assets AS OF SNAPSHOT 42' AS text -- FROM public.assets AS OF SNAPSHOT 7\nFROM public.assets";

        assert_eq!(rewrite_as_of_snapshot_selectors(sql), None);
    }

    #[test]
    fn rewrites_as_of_timestamp_table_selector() {
        let sql = "SELECT id FROM public.assets AS OF TIMESTAMP '2026-07-09T12:00:00Z'";

        assert_eq!(
            rewrite_as_of_snapshot_selectors(sql).as_deref(),
            Some("SELECT id FROM public.assets(snapshot_at => '2026-07-09T12:00:00Z')")
        );
    }

    #[test]
    fn leaves_non_literal_as_of_timestamp_for_parser_error() {
        let sql = "SELECT id FROM public.assets AS OF TIMESTAMP now()";

        assert_eq!(rewrite_as_of_snapshot_selectors(sql), None);
    }
}
