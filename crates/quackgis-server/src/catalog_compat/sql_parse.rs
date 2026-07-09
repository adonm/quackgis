// SPDX-License-Identifier: Apache-2.0
//! Small SQL-shape parsing helpers for trace-derived catalog shims.

pub(super) fn strip_trailing_semicolon(sql: &str) -> &str {
    sql.trim().trim_end_matches(';').trim_end()
}

pub(super) fn select_items(sql: &str) -> Vec<String> {
    let Some(select_start) = sql.to_lowercase().find("select ").map(|idx| idx + 7) else {
        return Vec::new();
    };
    let lower = sql.to_lowercase();
    let Some(from_end) = lower[select_start..]
        .find(" from ")
        .map(|idx| select_start + idx)
    else {
        return Vec::new();
    };
    split_top_level_commas(&sql[select_start..from_end])
}

fn split_top_level_commas(select_list: &str) -> Vec<String> {
    let mut items = Vec::new();
    let mut depth = 0_i32;
    let mut in_quotes = false;
    let mut start = 0_usize;
    for (idx, ch) in select_list.char_indices() {
        match ch {
            '"' => in_quotes = !in_quotes,
            '(' if !in_quotes => depth += 1,
            ')' if !in_quotes => depth -= 1,
            ',' if !in_quotes && depth == 0 => {
                items.push(select_list[start..idx].trim().to_string());
                start = idx + ch.len_utf8();
            }
            _ => {}
        }
    }
    if start < select_list.len() {
        items.push(select_list[start..].trim().to_string());
    }
    items
}

pub(super) fn select_item_output_name(item: &str) -> Option<String> {
    let lower = item.to_lowercase();
    if let Some(as_pos) = lower.rfind(" as ") {
        return Some(trim_identifier(&item[as_pos + 4..]));
    }
    item.rsplit('.')
        .next()
        .map(trim_identifier)
        .filter(|name| !name.is_empty())
}

fn trim_identifier(identifier: &str) -> String {
    identifier
        .trim()
        .trim_matches('"')
        .trim_matches('`')
        .to_string()
}

pub(super) fn escape_identifier(identifier: &str) -> String {
    identifier.replace('"', "\"\"")
}

pub(super) fn count_positional_placeholders(sql: &str) -> usize {
    let bytes = sql.as_bytes();
    let mut max_placeholder = 0_usize;
    let mut anonymous_placeholders = 0_usize;
    let mut i = 0_usize;
    while i < bytes.len() {
        match bytes[i] {
            b'\'' => i = skip_single_quoted(bytes, i + 1),
            b'"' => i = skip_double_quoted(bytes, i + 1),
            b'-' if bytes.get(i + 1) == Some(&b'-') => i = skip_line_comment(bytes, i + 2),
            b'/' if bytes.get(i + 1) == Some(&b'*') => i = skip_block_comment(bytes, i + 2),
            b'$' => {
                if let Some(end) = dollar_quoted_literal_end(bytes, i) {
                    i = end;
                    continue;
                }
                if !bytes.get(i + 1).is_some_and(u8::is_ascii_digit) {
                    i += 1;
                    continue;
                }

                i += 1;
                let start = i;
                while i < bytes.len() && bytes[i].is_ascii_digit() {
                    i += 1;
                }
                if let Ok(idx) = sql[start..i].parse::<usize>() {
                    max_placeholder = max_placeholder.max(idx);
                }
            }
            b'?' => {
                anonymous_placeholders += 1;
                i += 1;
            }
            _ => i += 1,
        }
    }
    max_placeholder.max(anonymous_placeholders)
}

fn skip_single_quoted(bytes: &[u8], mut i: usize) -> usize {
    while i < bytes.len() {
        if bytes[i] == b'\'' {
            if bytes.get(i + 1) == Some(&b'\'') {
                i += 2;
            } else {
                return i + 1;
            }
        } else {
            i += 1;
        }
    }
    i
}

fn skip_double_quoted(bytes: &[u8], mut i: usize) -> usize {
    while i < bytes.len() {
        if bytes[i] == b'"' {
            if bytes.get(i + 1) == Some(&b'"') {
                i += 2;
            } else {
                return i + 1;
            }
        } else {
            i += 1;
        }
    }
    i
}

fn skip_line_comment(bytes: &[u8], mut i: usize) -> usize {
    while i < bytes.len() && bytes[i] != b'\n' {
        i += 1;
    }
    i
}

fn skip_block_comment(bytes: &[u8], mut i: usize) -> usize {
    while i + 1 < bytes.len() {
        if bytes[i] == b'*' && bytes[i + 1] == b'/' {
            return i + 2;
        }
        i += 1;
    }
    bytes.len()
}

fn dollar_quoted_literal_end(bytes: &[u8], start: usize) -> Option<usize> {
    if bytes.get(start) != Some(&b'$') {
        return None;
    }
    let mut tag_end = start + 1;
    while tag_end < bytes.len() && bytes[tag_end] != b'$' {
        if !bytes[tag_end].is_ascii_alphanumeric() && bytes[tag_end] != b'_' {
            return None;
        }
        tag_end += 1;
    }
    if tag_end >= bytes.len() {
        return None;
    }

    let delimiter = &bytes[start..=tag_end];
    let mut i = tag_end + 1;
    while i + delimiter.len() <= bytes.len() {
        if bytes[i..].starts_with(delimiter) {
            return Some(i + delimiter.len());
        }
        i += 1;
    }
    Some(bytes.len())
}

pub(super) fn parse_single_quoted_literal(sql: &str) -> Option<String> {
    let start = sql.find('\'')? + 1;
    let mut out = String::new();
    let bytes = sql.as_bytes();
    let mut i = start;
    while i < bytes.len() {
        if bytes[i] == b'\'' {
            if bytes.get(i + 1) == Some(&b'\'') {
                out.push('\'');
                i += 2;
            } else {
                return Some(out);
            }
        } else {
            out.push(bytes[i] as char);
            i += 1;
        }
    }
    None
}

pub(super) fn parse_first_u32(s: &str) -> Option<u32> {
    let digits = s
        .chars()
        .skip_while(|ch| !ch.is_ascii_digit())
        .take_while(|ch| ch.is_ascii_digit())
        .collect::<String>();
    digits.parse().ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn count_placeholders_uses_highest_numbered_parameter() {
        assert_eq!(count_positional_placeholders("SELECT $2, $10, $1"), 10);
    }

    #[test]
    fn count_placeholders_counts_anonymous_parameters() {
        assert_eq!(count_positional_placeholders("SELECT ? + ?"), 2);
    }

    #[test]
    fn count_placeholders_ignores_literals_identifiers_and_comments() {
        assert_eq!(
            count_positional_placeholders(
                "SELECT '$1 ? '' $2', \"$3?\" FROM t \
                 -- ignored $4 ?\n\
                 WHERE a = $2 AND b = ? /* ignored $5 ? */"
            ),
            2
        );
    }

    #[test]
    fn count_placeholders_ignores_dollar_quoted_literals() {
        assert_eq!(
            count_positional_placeholders("SELECT $$ $1 ? $$, $tag$ $2 ? $tag$ WHERE id = $3"),
            3
        );
    }
}
