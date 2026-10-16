//! Merging the Lua tables in client files several layers carry, such as
//! `System/OngoingQuestInfoList_True.lub` or `System/itemInfo.lua`, where
//! each mod adds its own quests or items to one table.
//!
//! The files are mostly tables assigned at the top (`tbl = { [501] = {...},
//! ... }`), often followed by the functions the client calls. A later file's
//! keyed entries (`[501] = ...`, `["name"] = ...`, `name = ...`) replace the
//! earlier file's entry with the same key or are added at the end of its
//! table, and a table the earlier file lacks is added at the end of the file.
//! Everything else comes from the earlier file as it is, so its functions
//! stay and the later file's are dropped.
//!
//! The text is handled as bytes: client files are often in the Korean code
//! page (CP949), whose second bytes never look like a quote, a backslash or
//! a bracket. Compiled files (Lua bytecode) can't be merged.

/// A keyed entry of the later file, and what happened to it.
#[derive(Debug, PartialEq)]
pub enum Outcome {
    /// The earlier file didn't have the key.
    Added,
    /// The earlier file had the same value.
    Same,
    /// The earlier file had another value, which this one replaced.
    Replaced,
}

/// One keyed entry of the later file: its table, its key and its outcome.
pub type Entry = (String, String, Outcome);

/// Why a file can't be merged.
pub fn unmergeable(text: &[u8]) -> Option<&'static str> {
    if text.starts_with(b"\x1bLua") {
        Some("it's compiled Lua")
    } else if parse(text).is_none_or(|d| d.tables.is_empty()) {
        Some("no tables could be read from it")
    } else {
        None
    }
}

/// The keyed entries of the file's tables, as (table, key).
pub fn keys(text: &[u8]) -> Vec<(String, String)> {
    let Some(doc) = parse(text) else { return Vec::new() };
    doc.tables
        .iter()
        .flat_map(|t| t.entries.iter().filter_map(|e| Some((t.name.clone(), e.key.clone()?))))
        .collect()
}

/// `base` with `add`'s tables merged in, and what happened to each of
/// `add`'s keyed entries. None when either can't be read or `add` has no
/// tables.
pub fn merge(base: &[u8], add: &[u8]) -> Option<(Vec<u8>, Vec<Entry>)> {
    let b = parse(base)?;
    let a = parse(add)?;
    if a.tables.is_empty() {
        return None;
    }
    let nl: &[u8] = if base.windows(2).any(|w| w == b"\r\n") { b"\r\n" } else { b"\n" };
    let mut edits: Vec<(usize, usize, Vec<u8>)> = Vec::new();
    let mut tail = Vec::new();
    let mut outcomes = Vec::new();
    for at in &a.tables {
        // A table assigned twice (`QuestInfoList = {}` first, as kRO's
        // files do) ends up as the last assignment.
        let Some(bt) = b.tables.iter().rev().find(|t| t.name == at.name) else {
            tail.extend_from_slice(nl);
            tail.extend_from_slice(&add[at.start..at.end]);
            tail.extend_from_slice(nl);
            outcomes.extend(at.entries.iter().filter_map(|e| Some((at.name.clone(), e.key.clone()?, Outcome::Added))));
            continue;
        };
        let mut inserts = Vec::new();
        for ae in &at.entries {
            let found = ae.key.as_ref().and_then(|k| bt.entries.iter().rev().find(|be| be.key.as_ref() == Some(k)));
            match (found, &ae.key) {
                (Some(be), Some(key)) => {
                    let outcome = if b.value(be) == a.value(ae) {
                        Outcome::Same
                    } else {
                        edits.retain(|(start, _, _)| *start != be.start);
                        edits.push((be.start, be.end, add[ae.start..ae.end].to_vec()));
                        Outcome::Replaced
                    };
                    outcomes.push((at.name.clone(), key.clone(), outcome));
                }
                _ => {
                    // An entry without a key is added unless it's there already.
                    if ae.key.is_none() && bt.entries.iter().any(|be| be.key.is_none() && b.value(be) == a.value(ae)) {
                        continue;
                    }
                    if let Some(key) = &ae.key {
                        outcomes.push((at.name.clone(), key.clone(), Outcome::Added));
                    }
                    inserts.push(&add[ae.start..ae.end]);
                }
            }
        }
        if inserts.is_empty() {
            continue;
        }
        let indent = bt.entries.last().map_or(&b"\t"[..], |e| indent(base, e.start));
        let comma = bt.entries.last().is_some_and(|e| !e.separated);
        let own_line = line_start(base, bt.close);
        if let (true, Some(last)) = (comma, bt.entries.last()) {
            // Once, though a mod may assign the same table more than once.
            let edit = (last.end, last.end, b",".to_vec());
            if !edits.contains(&edit) {
                edits.push(edit);
            }
        }
        let mut text = Vec::new();
        if own_line.is_none() {
            text.extend_from_slice(nl);
        }
        for entry in inserts {
            text.extend_from_slice(indent);
            text.extend_from_slice(entry);
            text.push(b',');
            text.extend_from_slice(nl);
        }
        let at = own_line.unwrap_or(bt.close);
        edits.push((at, at, text));
    }
    // In order; of two at one place, the one added later goes after the other.
    let mut edits: Vec<_> = edits.into_iter().enumerate().collect();
    edits.sort_by_key(|(i, (start, end, _))| (*start, *end, *i));
    let mut out = Vec::with_capacity(base.len() + add.len());
    let mut at = 0;
    for (_, (start, end, text)) in edits {
        out.extend_from_slice(&base[at..start]);
        out.extend_from_slice(&text);
        at = end;
    }
    out.extend_from_slice(&base[at..]);
    if !tail.is_empty() {
        if !out.is_empty() && !out.ends_with(b"\n") {
            out.extend_from_slice(nl);
        }
        out.extend_from_slice(&tail);
    }
    Some((out, outcomes))
}

/// Where the line holding `at` starts, when only spaces come before it.
fn line_start(text: &[u8], at: usize) -> Option<usize> {
    let mut i = at;
    while i > 0 && matches!(text[i - 1], b' ' | b'\t') {
        i -= 1;
    }
    (i == 0 || text[i - 1] == b'\n').then_some(i)
}

/// The spaces before `at` on its line, or a tab when something else comes first.
fn indent(text: &[u8], at: usize) -> &[u8] {
    match line_start(text, at) {
        Some(start) => &text[start..at],
        None => b"\t",
    }
}

#[derive(Clone, Copy, PartialEq, Debug)]
enum Kind {
    Word,
    Str,
    /// `==`, `~=`, `<=` or `>=`, so they aren't taken for `=`.
    Compare,
    Punct(u8),
}

#[derive(Clone, Copy, Debug)]
struct Tok {
    kind: Kind,
    start: usize,
    end: usize,
}

struct Doc<'a> {
    text: &'a [u8],
    toks: Vec<Tok>,
    tables: Vec<Table>,
}

impl Doc<'_> {
    /// An entry's value as its tokens, so spacing and comments don't count.
    fn value(&self, e: &TableEntry) -> Vec<&[u8]> {
        self.toks[e.value..=e.last].iter().map(|t| &self.text[t.start..t.end]).collect()
    }
}

/// A table assigned at the top of the file.
struct Table {
    name: String,
    /// From the name to the closing brace.
    start: usize,
    end: usize,
    /// The closing brace.
    close: usize,
    entries: Vec<TableEntry>,
}

struct TableEntry {
    key: Option<String>,
    start: usize,
    end: usize,
    /// Its value's first and last tokens.
    value: usize,
    last: usize,
    /// Followed by `,` or `;`.
    separated: bool,
}

/// The opening `[` of a long bracket (`[[`, `[==[`) at `i`, and its level.
fn long_open(s: &[u8], i: usize) -> Option<usize> {
    if s.get(i) != Some(&b'[') {
        return None;
    }
    let mut j = i + 1;
    while s.get(j) == Some(&b'=') {
        j += 1;
    }
    (s.get(j) == Some(&b'[')).then_some(j - i - 1)
}

/// Just past the long bracket of `level` opened at `i`.
fn long_close(s: &[u8], i: usize, level: usize) -> Option<usize> {
    let mut close = vec![b']'];
    close.extend(std::iter::repeat_n(b'=', level));
    close.push(b']');
    let from = i + level + 2;
    s.get(from..)?.windows(close.len()).position(|w| w == close).map(|p| from + p + close.len())
}

fn lex(s: &[u8]) -> Option<Vec<Tok>> {
    let mut toks = Vec::new();
    let mut i = if s.starts_with(b"\xef\xbb\xbf") { 3 } else { 0 };
    while i < s.len() {
        let c = s[i];
        let start = i;
        let kind = match c {
            b' ' | b'\t' | b'\r' | b'\n' | 0x0b | 0x0c => {
                i += 1;
                continue;
            }
            b'-' if s.get(i + 1) == Some(&b'-') => {
                i += 2;
                match long_open(s, i) {
                    Some(level) => i = long_close(s, i, level)?,
                    None => {
                        while i < s.len() && s[i] != b'\n' {
                            i += 1;
                        }
                    }
                }
                continue;
            }
            b'"' | b'\'' => {
                i += 1;
                loop {
                    let b = *s.get(i)?;
                    i += 1;
                    match b {
                        b'\\' => i += 1,
                        b'\n' => return None,
                        _ if b == c => break,
                        _ => {}
                    }
                }
                Kind::Str
            }
            b'[' if long_open(s, i).is_some() => {
                i = long_close(s, i, long_open(s, i)?)?;
                Kind::Str
            }
            b'=' | b'~' | b'<' | b'>' if s.get(i + 1) == Some(&b'=') => {
                i += 2;
                Kind::Compare
            }
            _ if c.is_ascii_alphanumeric() || c == b'_' => {
                let number = c.is_ascii_digit();
                while i < s.len() && (s[i].is_ascii_alphanumeric() || s[i] == b'_' || (number && s[i] == b'.')) {
                    i += 1;
                }
                Kind::Word
            }
            _ => {
                i += 1;
                Kind::Punct(c)
            }
        };
        toks.push(Tok { kind, start, end: i });
    }
    Some(toks)
}

fn word<'a>(text: &'a [u8], t: &Tok) -> Option<&'a [u8]> {
    (t.kind == Kind::Word).then(|| &text[t.start..t.end])
}

fn parse(text: &[u8]) -> Option<Doc<'_>> {
    if text.starts_with(b"\x1bLua") {
        return None;
    }
    let toks = lex(text)?;
    let mut tables = Vec::new();
    // Brackets, and the blocks `function`, `do`, `if` and `repeat` open.
    let (mut brackets, mut blocks) = (0i32, 0i32);
    let mut i = 0;
    while i < toks.len() {
        if brackets == 0 && blocks == 0 {
            if let Some((table, next)) = top_table(text, &toks, i) {
                tables.push(table);
                i = next;
                continue;
            }
        }
        let t = toks[i];
        match t.kind {
            Kind::Punct(b'{' | b'(' | b'[') => brackets += 1,
            Kind::Punct(b'}' | b')' | b']') => brackets -= 1,
            Kind::Word => match &text[t.start..t.end] {
                b"function" | b"do" | b"if" | b"repeat" => blocks += 1,
                b"end" | b"until" => blocks -= 1,
                _ => {}
            },
            _ => {}
        }
        if brackets < 0 || blocks < 0 {
            return None;
        }
        i += 1;
    }
    (brackets == 0 && blocks == 0).then_some(Doc { text, toks, tables })
}

/// A table assignment starting at token `i` (`[local] name[.name] = {`),
/// and the token after it.
fn top_table(text: &[u8], toks: &[Tok], mut i: usize) -> Option<(Table, usize)> {
    let start = toks[i].start;
    if word(text, &toks[i]) == Some(b"local") {
        i += 1;
    }
    let first = word(text, toks.get(i)?)?;
    if first[0].is_ascii_digit() || matches!(first, b"function" | b"return" | b"end") {
        return None;
    }
    let mut name = String::from_utf8_lossy(first).into_owned();
    i += 1;
    while toks.get(i)?.kind == Kind::Punct(b'.') {
        name.push('.');
        name.push_str(&String::from_utf8_lossy(word(text, toks.get(i + 1)?)?));
        i += 2;
    }
    if toks.get(i)?.kind != Kind::Punct(b'=') || toks.get(i + 1)?.kind != Kind::Punct(b'{') {
        return None;
    }
    let (entries, close) = table_entries(text, toks, i + 1)?;
    let close_tok = toks[close];
    Some((Table { name, start, end: close_tok.end, close: close_tok.start, entries }, close + 1))
}

/// The entries of the table whose `{` is token `open`, and its `}` token.
fn table_entries(text: &[u8], toks: &[Tok], open: usize) -> Option<(Vec<TableEntry>, usize)> {
    let mut entries = Vec::new();
    let mut depth = 0;
    let mut first: Option<usize> = None;
    let mut j = open + 1;
    loop {
        let t = *toks.get(j)?;
        if depth == 0 {
            match t.kind {
                Kind::Punct(b'}') => {
                    if let Some(f) = first {
                        entries.push(entry(text, toks, f, j - 1, false)?);
                    }
                    return Some((entries, j));
                }
                Kind::Punct(b',' | b';') => {
                    entries.push(entry(text, toks, first.take()?, j - 1, true)?);
                    j += 1;
                    continue;
                }
                _ => {
                    first.get_or_insert(j);
                }
            }
        }
        match t.kind {
            Kind::Punct(b'{' | b'(' | b'[') => depth += 1,
            Kind::Punct(b'}' | b')' | b']') => depth -= 1,
            _ => {}
        }
        if depth < 0 {
            return None;
        }
        j += 1;
    }
}

/// The entry from token `first` to `last`, with its key if it has one.
fn entry(text: &[u8], toks: &[Tok], first: usize, last: usize, separated: bool) -> Option<TableEntry> {
    let t = toks[first];
    let mut key = None;
    let mut value = first;
    if t.kind == Kind::Punct(b'[') {
        // `[key] =`: the matching `]` followed by `=`.
        let mut depth = 0;
        for k in first..=last {
            match toks[k].kind {
                Kind::Punct(b'[' | b'{' | b'(') => depth += 1,
                Kind::Punct(b']' | b'}' | b')') => depth -= 1,
                _ => {}
            }
            if depth == 0 {
                if k > first + 1 && k < last && toks[k + 1].kind == Kind::Punct(b'=') {
                    key = Some(key_text(text, &toks[first + 1..k]));
                    value = k + 2;
                }
                break;
            }
        }
    } else if t.kind == Kind::Word && first < last && toks[first + 1].kind == Kind::Punct(b'=') {
        key = Some(String::from_utf8_lossy(&text[t.start..t.end]).into_owned());
        value = first + 2;
    }
    if value > last {
        return None;
    }
    Some(TableEntry { key, start: t.start, end: toks[last].end, value, last, separated })
}

/// A key as one string: a quoted name without its quotes, anything else
/// as its tokens, so `[ 501 ]` and `[501]` are the same key.
fn key_text(text: &[u8], toks: &[Tok]) -> String {
    if let [t] = toks {
        let s = &text[t.start..t.end];
        if t.kind == Kind::Str && (s[0] == b'"' || s[0] == b'\'') && !s.contains(&b'\\') {
            return String::from_utf8_lossy(&s[1..s.len() - 1]).into_owned();
        }
    }
    let parts: Vec<String> = toks.iter().map(|t| String::from_utf8_lossy(&text[t.start..t.end]).into_owned()).collect();
    parts.join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn merged(base: &str, add: &str) -> (String, Vec<Entry>) {
        let (text, outcomes) = merge(base.as_bytes(), add.as_bytes()).unwrap();
        (String::from_utf8(text).unwrap(), outcomes)
    }

    const ITEM_INFO: &str = "tbl = {\n\t[501] = {\n\t\tidentifiedDisplayName = \"Red Potion\",\n\t\tslotCount = 0\n\t},\n\t[502] = {\n\t\tidentifiedDisplayName = \"Orange Potion\"\n\t}\n}\n\nfunction main()\n\tfor ItemID, DESC in pairs(tbl) do\n\t\tlocal x = { 1 }\n\tend\n\treturn true\nend\n";

    #[test]
    fn a_table_assigned_twice_gets_one_comma() {
        let (text, _) = merged("tbl = {\n\t[1] = { a = 1 }\n}\n", "tbl = { [2] = { b = 2 } }\ntbl = { [3] = { c = 3 } }\n");
        assert_eq!(text, "tbl = {\n\t[1] = { a = 1 },\n\t[2] = { b = 2 },\n\t[3] = { c = 3 },\n}\n");
    }

    #[test]
    fn entries_are_added_and_replaced_by_key() {
        let add = "tbl = {\n  [502] = { identifiedDisplayName = \"Better Orange\" },\n  [ 501 ] = {\n\t\tidentifiedDisplayName = \"Red Potion\", slotCount = 0 },\n  [30000] = { identifiedDisplayName = \"Eden Coin\" }\n}\nfunction main() return false end\n";
        let (text, outcomes) = merged(ITEM_INFO, add);
        assert_eq!(
            text,
            "tbl = {\n\t[501] = {\n\t\tidentifiedDisplayName = \"Red Potion\",\n\t\tslotCount = 0\n\t},\n\t[502] = { identifiedDisplayName = \"Better Orange\" },\n\t[30000] = { identifiedDisplayName = \"Eden Coin\" },\n}\n\nfunction main()\n\tfor ItemID, DESC in pairs(tbl) do\n\t\tlocal x = { 1 }\n\tend\n\treturn true\nend\n"
        );
        assert_eq!(
            outcomes,
            [
                ("tbl".into(), "502".into(), Outcome::Replaced),
                ("tbl".into(), "501".into(), Outcome::Same),
                ("tbl".into(), "30000".into(), Outcome::Added),
            ]
        );
        // The merged file reads back with every entry.
        assert_eq!(keys(text.as_bytes()).len(), 3);
    }

    #[test]
    fn a_new_table_goes_at_the_end() {
        let (text, outcomes) = merged("A = { x = 1 }", "B = {\r\n  [\"k\"] = 'v'; -- note\r\n}");
        assert_eq!(text, "A = { x = 1 }\n\nB = {\r\n  [\"k\"] = 'v'; -- note\r\n}\n");
        assert_eq!(outcomes, [("B".into(), "k".into(), Outcome::Added)]);
    }

    #[test]
    fn one_line_tables_and_line_endings() {
        let (text, _) = merged("QuestInfoList = { [1] = { Title = \"a\" } }\r\n", "QuestInfoList = { [2] = { Title = \"b\" } }");
        assert_eq!(text, "QuestInfoList = { [1] = { Title = \"a\" }, \r\n\t[2] = { Title = \"b\" },\r\n}\r\n");
        let (text, _) = merged("t = {}", "t = { 'x' }");
        assert_eq!(text, "t = {\n\t'x',\n}");
        let (text, _) = merged("t = {[1]=2}", "t = {[3]=4}");
        assert_eq!(text, "t = {[1]=2,\n\t[3]=4,\n}");
    }

    #[test]
    fn strings_and_comments_dont_confuse_it() {
        let base = "--[[ tbl = { [1] = 1 } ]]\nt = {\n\t[1] = \"}\\\" {\", -- }\n\t[2] = [==[ ] } ]==],\n}\n";
        let (_, outcomes) = merged(base, "t = { [2] = [==[ ] } ]==], [1] = \"x\" }");
        assert_eq!(outcomes, [("t".into(), "2".into(), Outcome::Same), ("t".into(), "1".into(), Outcome::Replaced)]);
        assert_eq!(keys(base.as_bytes()), [("t".to_owned(), "1".to_owned()), ("t".to_owned(), "2".to_owned())]);
    }

    #[test]
    fn korean_code_page_text_passes_through() {
        // "빨간포션" in CP949, as kRO's own files spell names.
        let base = b"tbl = {\n\t[501] = { name = \"\xbb\xa1\xb0\xa3\xc6\xf7\xbc\xc7\" },\n}\n".to_vec();
        let (text, outcomes) = merge(&base, b"tbl = { [502] = { name = \"\xc6\xf7\xbc\xc7\" } }").unwrap();
        assert_eq!(outcomes, [("tbl".into(), "502".into(), Outcome::Added)]);
        assert!(text.windows(4).any(|w| w == b"\xc6\xf7\xbc\xc7"));
        assert_eq!(keys(&text).len(), 2);
    }

    #[test]
    fn the_last_assignment_of_a_table_is_the_one_added_to() {
        let (text, _) = merged("Q = {}\nQ = {\n\t[1] = 1,\n}\n", "Q = { [2] = 2 }");
        assert_eq!(text, "Q = {}\nQ = {\n\t[1] = 1,\n\t[2] = 2,\n}\n");
    }

    /// Merges real files: `LUB_BASE`, then each of `LUB_ADD` (`:`-separated)
    /// into `LUB_OUT`, printing the outcomes' counts.
    #[test]
    #[ignore]
    fn real_files() {
        let var = |n| std::env::var(n).unwrap();
        let mut text = std::fs::read(var("LUB_BASE")).unwrap();
        for add in var("LUB_ADD").split(':') {
            let (next, outcomes) = merge(&text, &std::fs::read(add).unwrap()).unwrap();
            let n = |o: Outcome| outcomes.iter().filter(|e| e.2 == o).count();
            println!("{add}: {} added, {} same, {} replaced", n(Outcome::Added), n(Outcome::Same), n(Outcome::Replaced));
            text = next;
        }
        std::fs::write(var("LUB_OUT"), text).unwrap();
    }

    #[test]
    fn what_cant_be_merged() {
        assert_eq!(unmergeable(b"\x1bLuaQ\x00\x01"), Some("it's compiled Lua"));
        assert_eq!(unmergeable(b"function main() end"), Some("no tables could be read from it"));
        assert_eq!(unmergeable(b"t = { [1] = 1"), Some("no tables could be read from it"));
        assert_eq!(unmergeable(ITEM_INFO.as_bytes()), None);
        assert!(merge(ITEM_INFO.as_bytes(), b"function main() end").is_none());
        // Tables inside functions aren't the file's tables.
        assert_eq!(keys(b"function f()\n t = { a = 1 }\nend\nu = { b = 2 }"), [("u".to_owned(), "b".to_owned())]);
    }
}
