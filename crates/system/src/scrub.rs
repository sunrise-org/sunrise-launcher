//! Takes out of text what could say who or where a player is, before it is kept to be sent:
//! the user folder's name in paths, the user name, e-mail and IP addresses, UUIDs (accounts),
//! and the terms the caller knows are private (world names, server addresses, account names).

/// What replaces each kind of match.
pub const USER: &str = "<user>";
pub const EMAIL: &str = "<email>";
pub const IP: &str = "<ip>";
pub const UUID: &str = "<uuid>";
pub const PRIVATE: &str = "<private>";

#[derive(Clone, Debug, Default)]
pub struct Scrubber {
    /// Normalized path roots to hide across path aliases (before IP rewriting), longest first.
    paths: Vec<String>,
    /// Terms to hide wherever they appear (any case), longest first.
    terms: Vec<String>,
    /// Whole-word terms to hide (any case), longest first — used for 1–2 byte profile names.
    word_terms: Vec<String>,
    user: Option<String>,
}

impl Scrubber {
    /// A scrubber that also knows this machine's user name.
    pub fn new() -> Self {
        let user = ["USER", "USERNAME", "LOGNAME"]
            .iter()
            .find_map(|k| std::env::var(k).ok().filter(|u| u.trim().len() >= 3));
        Self {
            paths: Vec::new(),
            terms: Vec::new(),
            word_terms: Vec::new(),
            user,
        }
    }

    /// Without the machine's user name (tests).
    pub fn bare() -> Self {
        Self::default()
    }

    pub fn with_user(mut self, user: impl Into<String>) -> Self {
        self.user = Some(user.into()).filter(|u| u.trim().len() >= 3);
        self
    }

    /// Hides a filesystem root across separator, case, JSON-escape, and Windows verbatim aliases,
    /// before IP rewriting so an IP-bearing root still matches whole.
    pub fn path_term(mut self, path: impl Into<String>) -> Self {
        if let Some(norm) = normalize_path_root(&path.into())
            && !self
                .paths
                .iter()
                .any(|existing| existing.to_lowercase() == norm.to_lowercase())
        {
            self.paths.push(norm);
            self.paths.sort_by_key(|p| std::cmp::Reverse(p.len()));
        }
        self
    }

    /// Hides `term` wherever it appears (any case). Terms under 3 characters are ignored: they
    /// would blank out ordinary words. Absolute path roots are routed through [`Self::path_term`].
    pub fn term(mut self, term: impl Into<String>) -> Self {
        let term = term.into();
        if looks_like_path_root(&term) {
            return self.path_term(term);
        }
        if term.trim().len() >= 3
            && !self
                .terms
                .iter()
                .any(|existing| existing.to_lowercase() == term.to_lowercase())
        {
            self.terms.push(term);
            self.terms.sort_by_key(|t| std::cmp::Reverse(t.len()));
        }
        self
    }

    /// Hides whole-word occurrences of `term` (any case), including 1–2 character names that
    /// would blank out ordinary words if matched as substrings.
    pub fn word_term(mut self, term: impl Into<String>) -> Self {
        let term = term.into();
        let trimmed = term.trim();
        if !trimmed.is_empty()
            && !self
                .word_terms
                .iter()
                .any(|existing| existing.to_lowercase() == trimmed.to_lowercase())
        {
            self.word_terms.push(trimmed.to_owned());
            self.word_terms.sort_by_key(|t| std::cmp::Reverse(t.len()));
        }
        self
    }

    pub fn scrub(&self, text: &str) -> String {
        let mut out = text.to_owned();
        // Registered path roots run before IP rewriting so an IP-bearing root (such as
        // `/mnt/10.0.0.5/sunrise` or `\\10.0.0.5\share\sunrise`) still matches whole.
        for path in &self.paths {
            out = replace_path(&out, path, PRIVATE);
        }
        out = replace_tokens(&out, email_at);
        out = replace_tokens(&out, ipv4_at);
        out = replace_tokens(&out, ipv6_at);
        out = replace_tokens(&out, uuid_at);
        out = connecting_host(&out);
        // Private terms can be part of the text that identifies an address or credential.
        for term in &self.terms {
            out = replace_term(&out, term, PRIVATE);
        }
        for word in &self.word_terms {
            out = replace_word(&out, word, PRIVATE);
        }
        out = user_folders(&out);
        if let Some(user) = &self.user {
            out = replace_word(&out, user, USER);
        }
        out
    }
}

/// Hides the host in a connection record, keeping the port: vanilla writes `host, port`,
/// and launcher connection lines may use `host:port`. Ordinary prose is left alone.
fn connecting_host(text: &str) -> String {
    const HEAD: &str = "Connecting to ";
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(at) = rest.find(HEAD) {
        let from = at + HEAD.len();
        out.push_str(&rest[..from]);
        let after = &rest[from..];
        let line = after.find(['\r', '\n']).unwrap_or(after.len());
        let address = after[..line].trim_end();
        let host = address
            .rsplit_once(", ")
            .or_else(|| address.rsplit_once(':'))
            .and_then(|(host, port)| {
                (!host.is_empty()
                    && !host.contains(char::is_whitespace)
                    && (port == PRIVATE
                        || (!port.is_empty() && port.bytes().all(|b| b.is_ascii_digit()))))
                .then_some(host)
            });
        match host {
            Some(host) => {
                out.push_str(if host == IP || host == PRIVATE {
                    host
                } else {
                    PRIVATE
                });
                rest = &after[host.len()..];
            }
            None => rest = after,
        }
    }
    out.push_str(rest);
    out
}

/// `/Users/ana/…`, `/home/ana/…`, `C:\Users\Ana Silva\…` (also JSON-escaped): the folder after
/// `Users` or `home` becomes [`USER`], the rest of the path stays. Windows names have spaces, so
/// the name runs to the next separator, quote, bracket or line end.
fn user_folders(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    'scan: while !rest.is_empty() {
        for parent in ["Users", "home"] {
            for sep in ["/", "\\\\", "\\"] {
                let head = format!("{sep}{parent}{sep}");
                if let Some(after) = rest.strip_prefix(head.as_str()) {
                    let name_len = after
                        .find(|c: char| {
                            matches!(
                                c,
                                '/' | '\\' | '"' | '\'' | '\n' | '\r' | ')' | ']' | ',' | ';'
                            )
                        })
                        .unwrap_or(after.len());
                    out.push_str(&head);
                    if name_len > 0 {
                        out.push_str(USER);
                    }
                    rest = &after[name_len..];
                    continue 'scan;
                }
            }
        }
        let c = rest.chars().next().unwrap_or_default();
        out.push(c);
        rest = &rest[c.len_utf8()..];
    }
    out
}

/// Replaces every match of `matcher` (which returns the length of a match starting at a byte
/// offset, if one does) with its label.
fn replace_tokens(
    text: &str,
    matcher: fn(&[u8], usize) -> Option<(usize, &'static str)>,
) -> String {
    let bytes = text.as_bytes();
    let mut out = String::with_capacity(text.len());
    let mut i = 0;
    let mut copied = 0;
    while i < bytes.len() {
        if text.is_char_boundary(i)
            && let Some((len, label)) = matcher(bytes, i)
        {
            out.push_str(&text[copied..i]);
            out.push_str(label);
            i += len;
            copied = i;
            continue;
        }
        i += 1;
    }
    out.push_str(&text[copied..]);
    out
}

fn word_byte(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_'
}

/// Whether a token may start at `i`: not in the middle of a word, number or address.
fn starts_token(bytes: &[u8], i: usize) -> bool {
    i == 0 || !(word_byte(bytes[i - 1]) || matches!(bytes[i - 1], b'.' | b':' | b'-' | b'@'))
}

fn ends_token(bytes: &[u8], end: usize) -> bool {
    end == bytes.len() || !word_byte(bytes[end])
}

fn email_at(bytes: &[u8], i: usize) -> Option<(usize, &'static str)> {
    let local = |b: u8| word_byte(b) || matches!(b, b'.' | b'%' | b'+' | b'-');
    if !starts_token(bytes, i) || !local(bytes[i]) {
        return None;
    }
    let at = i + bytes[i..].iter().position(|&b| !local(b))?;
    if bytes.get(at) != Some(&b'@') {
        return None;
    }
    let domain = |b: u8| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'-');
    let end = at + 1 + bytes[at + 1..].iter().take_while(|&&b| domain(b)).count();
    let host = &bytes[at + 1..end];
    let dot = host.iter().rposition(|&b| b == b'.')?;
    (dot > 0 && host.len() - dot > 2).then_some((end - i, EMAIL))
}

fn ipv4_at(bytes: &[u8], i: usize) -> Option<(usize, &'static str)> {
    if !starts_token(bytes, i) {
        return None;
    }
    let mut end = i;
    for part in 0..4 {
        if part > 0 {
            if bytes.get(end) != Some(&b'.') {
                return None;
            }
            end += 1;
        }
        let digits = bytes[end..]
            .iter()
            .take_while(|b| b.is_ascii_digit())
            .count();
        if digits == 0 || digits > 3 {
            return None;
        }
        let n: u32 = std::str::from_utf8(&bytes[end..end + digits])
            .ok()?
            .parse()
            .ok()?;
        if n > 255 {
            return None;
        }
        end += digits;
    }
    // A fifth part makes it a version number (`10.0.19045.1`), not an address.
    if bytes.get(end) == Some(&b'.') && bytes.get(end + 1).is_some_and(u8::is_ascii_digit) {
        return None;
    }
    // A port goes with it.
    if bytes.get(end) == Some(&b':') {
        let digits = bytes[end + 1..]
            .iter()
            .take_while(|b| b.is_ascii_digit())
            .count();
        if (1..=5).contains(&digits) {
            end += 1 + digits;
        }
    }
    ends_token(bytes, end).then_some((end - i, IP))
}

/// Eight hex groups with colons, or fewer around a `::` (at least three groups, so a Rust path
/// such as `a::b` is left alone).
fn ipv6_at(bytes: &[u8], i: usize) -> Option<(usize, &'static str)> {
    if !starts_token(bytes, i) {
        return None;
    }
    let len = bytes[i..]
        .iter()
        .take_while(|b| b.is_ascii_hexdigit() || **b == b':')
        .count();
    let token = std::str::from_utf8(&bytes[i..i + len]).ok()?;
    let token = token.trim_end_matches(':');
    let groups: Vec<&str> = token.split(':').collect();
    let filled = groups.iter().filter(|g| !g.is_empty()).count();
    let ok = groups.iter().all(|g| g.len() <= 4)
        && (filled == 8 || (token.contains("::") && filled >= 3))
        && groups.iter().any(|g| g.bytes().any(|b| b.is_ascii_digit()));
    (ok && ends_token(bytes, i + token.len())).then_some((token.len(), IP))
}

/// `0f8c…-…` (8-4-4-4-12) or the same 32 hex digits without dashes, as accounts are named.
fn uuid_at(bytes: &[u8], i: usize) -> Option<(usize, &'static str)> {
    if !starts_token(bytes, i) {
        return None;
    }
    let hex = |range: std::ops::Range<usize>| {
        bytes
            .get(range)
            .is_some_and(|s| s.iter().all(u8::is_ascii_hexdigit))
    };
    let dashed = [8usize, 13, 18, 23]
        .iter()
        .all(|&d| bytes.get(i + d) == Some(&b'-'))
        && [(0, 8), (9, 13), (14, 18), (19, 23), (24, 36)]
            .iter()
            .all(|&(a, b)| hex(i + a..i + b));
    if dashed && ends_token(bytes, i + 36) {
        return Some((36, UUID));
    }
    (hex(i..i + 32) && ends_token(bytes, i + 32)).then_some((32, UUID))
}

/// Lowercased views of `text` and `needle` when their byte boundaries align with `text`,
/// falling back to ASCII-only folding for ASCII needles and exact matching otherwise.
fn match_slices<'a>(
    text: &'a str,
    needle: &'a str,
) -> (std::borrow::Cow<'a, str>, std::borrow::Cow<'a, str>) {
    let lower = text.to_lowercase();
    let lower_needle = needle.to_lowercase();
    // Lowercasing can change byte lengths outside ASCII: then match with ASCII folding (which
    // preserves every boundary) or exactly instead. Equal total lengths aren't enough to stand one
    // in for the other: a character can grow (`İ` is 2 bytes and lowercases to 3) while another
    // shrinks (`K`, U+212A, is 3 and lowercases to 1), so the text keeps its length while its
    // boundaries move, and a match's offsets in `lower` would slice the text mid-character.
    let aligned = lower.len() == text.len()
        && lower_needle.len() == needle.len()
        && lower
            .char_indices()
            .map(|(at, _)| at)
            .eq(text.char_indices().map(|(at, _)| at));
    if aligned {
        return (
            std::borrow::Cow::Owned(lower),
            std::borrow::Cow::Owned(lower_needle),
        );
    }
    if needle.is_ascii() {
        return (
            std::borrow::Cow::Owned(text.to_ascii_lowercase()),
            std::borrow::Cow::Owned(needle.to_ascii_lowercase()),
        );
    }
    (
        std::borrow::Cow::Borrowed(text),
        std::borrow::Cow::Borrowed(needle),
    )
}

/// Whether `start..end` overlaps an existing scrubber placeholder (`<user>`, `<private>`, …).
fn inside_placeholder(bytes: &[u8], start: usize, end: usize) -> bool {
    const LABELS: [&[u8]; 6] = [
        USER.as_bytes(),
        EMAIL.as_bytes(),
        IP.as_bytes(),
        UUID.as_bytes(),
        PRIVATE.as_bytes(),
        b"<secret>",
    ];
    let search_from = start.saturating_sub(PRIVATE.len());
    bytes[search_from..end]
        .iter()
        .enumerate()
        .filter(|&(_, &b)| b == b'<')
        .any(|(rel, _)| {
            let open = search_from + rel;
            LABELS.iter().any(|label| {
                bytes[open..].starts_with(label) && start < open + label.len() && end > open
            })
        })
}

/// Substring occurrences of `term`, in any case, without rewriting scrubber placeholders.
fn replace_term(text: &str, term: &str, label: &str) -> String {
    if term.is_empty() {
        return text.to_owned();
    }
    let (hay, needle) = match_slices(text, term);
    let bytes = text.as_bytes();
    let mut out = String::with_capacity(text.len());
    let mut copied = 0;
    let mut from = 0;
    while let Some(found) = hay[from..].find(needle.as_ref()) {
        let start = from + found;
        let end = start + needle.len();
        if !inside_placeholder(bytes, start, end) {
            out.push_str(&text[copied..start]);
            out.push_str(label);
            copied = end;
        }
        from = end;
    }
    out.push_str(&text[copied..]);
    out
}

/// Whole-word occurrences of `word`, in any case.
fn replace_word(text: &str, word: &str, label: &str) -> String {
    if word.is_empty() {
        return text.to_owned();
    }
    let (hay, needle) = match_slices(text, word);
    let bytes = text.as_bytes();
    let mut out = String::with_capacity(text.len());
    let mut copied = 0;
    let mut from = 0;
    while let Some(found) = hay[from..].find(needle.as_ref()) {
        let start = from + found;
        let end = start + needle.len();
        let alone = (start == 0 || !word_byte(bytes[start - 1])) && ends_token(bytes, end);
        // A label put in earlier (`<user>` for a user named "user") stays as it is.
        if alone && !inside_placeholder(bytes, start, end) {
            out.push_str(&text[copied..start]);
            out.push_str(label);
            copied = end;
        }
        from = end;
    }
    out.push_str(&text[copied..]);
    out
}

fn looks_like_path_root(text: &str) -> bool {
    let bytes = text.as_bytes();
    text.starts_with('/')
        || text.starts_with(r"\\")
        || (bytes.len() >= 3
            && bytes[0].is_ascii_alphabetic()
            && bytes[1] == b':'
            && matches!(bytes[2], b'/' | b'\\'))
}

fn strip_verbatim_unc(raw: &str) -> Option<&str> {
    let rest = raw
        .strip_prefix(r"\\?\")
        .or_else(|| raw.strip_prefix(r"\\.\"))
        .or_else(|| raw.strip_prefix("//?/"))
        .or_else(|| raw.strip_prefix("//./"))?;
    let head = rest.as_bytes().get(..4)?;
    if head[..3].eq_ignore_ascii_case(b"UNC") && matches!(head[3], b'\\' | b'/') {
        Some(&rest[4..])
    } else {
        None
    }
}

fn collapse_slashes(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for ch in s.chars() {
        if ch != '/' || !out.ends_with('/') {
            out.push(ch);
        }
    }
    out
}

fn strip_dot_prefixes(mut s: &str) -> &str {
    while let Some(rest) = s.strip_prefix("./") {
        s = rest;
    }
    s
}

/// Normalizes a filesystem path root into one canonical `/`-separated form (`//server/share/...`
/// for UNC, `X:/...` for Windows drives, `/...` for Unix roots, `a/b/...` for multi-segment
/// relative roots). Bare roots with no normal path component return `None`.
fn normalize_path_root(raw: &str) -> Option<String> {
    let (unc, rest) = if let Some(tail) = strip_verbatim_unc(raw) {
        (true, tail)
    } else if let Some(tail) = raw
        .strip_prefix(r"\\?\")
        .or_else(|| raw.strip_prefix(r"\\.\"))
        .or_else(|| raw.strip_prefix("//?/"))
        .or_else(|| raw.strip_prefix("//./"))
    {
        (false, tail)
    } else if let Some(tail) = raw.strip_prefix(r"\\").or_else(|| raw.strip_prefix("//")) {
        (true, tail)
    } else {
        (false, raw)
    };
    let body = collapse_slashes(&rest.replace('\\', "/"));
    let body = body.trim_end_matches('/');
    if unc {
        let body = body.trim_start_matches('/');
        let (server, share) = body.split_once('/')?;
        if server.is_empty() || share.is_empty() {
            return None;
        }
        return Some(format!("//{body}"));
    }
    if body.is_empty() || body == "/" {
        return None;
    }
    let bytes = body.as_bytes();
    if bytes.len() >= 2 && bytes[0].is_ascii_alphabetic() && bytes[1] == b':' {
        let after_drive = body[2..].trim_start_matches('/');
        if after_drive.is_empty() {
            return None;
        }
        return Some(format!("{}:/{after_drive}", &body[..1]));
    }
    if let Some(after_root) = body.strip_prefix('/') {
        if after_root.is_empty() {
            return None;
        }
        return Some(format!("/{after_root}"));
    }
    // Relative path roots require a separator so single-word relative names stay in `self.terms`
    // (after `connecting_host`) rather than running ahead of address redaction.
    let clean = strip_dot_prefixes(body);
    if clean.len() < 3 || !clean.contains('/') {
        return None;
    }
    Some(clean.to_owned())
}

fn match_sep(bytes: &[u8], i: usize) -> Option<usize> {
    if !matches!(bytes.get(i), Some(b'/' | b'\\')) {
        return None;
    }
    let mut end = i;
    while matches!(bytes.get(end), Some(b'/' | b'\\')) {
        end += 1;
    }
    Some(end)
}

fn match_verbatim_prefix(bytes: &[u8], i: usize) -> Option<usize> {
    let first = *bytes.get(i)?;
    if !matches!(first, b'\\' | b'/') || bytes.get(i + 1) != Some(&first) {
        return None;
    }
    let after_lead = match_sep(bytes, i)?;
    if !matches!(bytes.get(after_lead), Some(b'?' | b'.')) {
        return None;
    }
    match_sep(bytes, after_lead + 1)
}

fn match_unc_prefix(bytes: &[u8], i: usize) -> Option<usize> {
    if let Some(after_verb) = match_verbatim_prefix(bytes, i) {
        let unc = bytes.get(after_verb..after_verb + 3)?;
        if unc.eq_ignore_ascii_case(b"UNC") {
            return match_sep(bytes, after_verb + 3);
        }
        return None;
    }
    let first = *bytes.get(i)?;
    if matches!(first, b'\\' | b'/') && bytes.get(i + 1) == Some(&first) {
        return match_sep(bytes, i);
    }
    None
}

fn match_segment_ci(text: &str, pos: usize, segment: &str) -> Option<usize> {
    if segment.is_ascii() {
        let end = pos + segment.len();
        return text
            .as_bytes()
            .get(pos..end)
            .filter(|slice| slice.eq_ignore_ascii_case(segment.as_bytes()))
            .map(|_| end);
    }
    let mut end = pos;
    let mut text_chars = text.get(pos..)?.chars();
    for expected in segment.chars() {
        let actual = text_chars.next()?;
        if !actual.to_lowercase().eq(expected.to_lowercase()) {
            return None;
        }
        end += actual.len_utf8();
    }
    Some(end)
}

fn match_path_body(text: &str, mut pos: usize, body: &str) -> Option<usize> {
    let bytes = text.as_bytes();
    for (index, segment) in body.split('/').enumerate() {
        if index > 0 {
            pos = match_sep(bytes, pos)?;
        }
        pos = match_segment_ci(text, pos, segment)?;
    }
    Some(pos)
}

fn path_match_end(text: &str, start: usize, root: &str) -> Option<usize> {
    let bytes = text.as_bytes();
    if let Some(body) = root.strip_prefix("//") {
        let pos = match_unc_prefix(bytes, start)?;
        return match_path_body(text, pos, body);
    }
    let rbytes = root.as_bytes();
    if rbytes.len() >= 3 && rbytes[0].is_ascii_alphabetic() && rbytes[1] == b':' {
        let pos = if let Some(after_verb) = match_verbatim_prefix(bytes, start) {
            after_verb
        } else if start > 0 && word_byte(bytes[start - 1]) {
            return None;
        } else {
            start
        };
        let drive = *bytes.get(pos)?;
        if !drive.eq_ignore_ascii_case(&rbytes[0]) || bytes.get(pos + 1) != Some(&b':') {
            return None;
        }
        let after_drive = match_sep(bytes, pos + 2)?;
        return match_path_body(text, after_drive, &root[3..]);
    }
    if let Some(body) = root.strip_prefix('/') {
        if bytes.get(start) != Some(&b'/') {
            return None;
        }
        return match_path_body(text, start + 1, body);
    }
    if start > 0 && word_byte(bytes[start - 1]) {
        return None;
    }
    let mut pos = start;
    if start == 0 || bytes[start - 1] != b'.' {
        while bytes.get(pos) == Some(&b'.') {
            let Some(after) = match_sep(bytes, pos + 1) else {
                break;
            };
            pos = after;
        }
    }
    match_path_body(text, pos, root)
}

/// Replaces every spelling of normalized `root` (separators, JSON escapes, case, Windows verbatim
/// prefixes) with `label`.
fn replace_path(text: &str, root: &str, label: &str) -> String {
    let bytes = text.as_bytes();
    let mut out = String::with_capacity(text.len());
    let mut copied = 0;
    let mut i = 0;
    while i < text.len() {
        if let Some(end) = path_match_end(text, i, root)
            && !inside_placeholder(bytes, i, end)
        {
            out.push_str(&text[copied..i]);
            out.push_str(label);
            copied = end;
            i = end;
            continue;
        }
        let ch = text[i..].chars().next().unwrap_or_default();
        i += ch.len_utf8();
    }
    out.push_str(&text[copied..]);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The game logs `Connecting to <host>, <port>` for every connection, as vanilla does: the
    /// host is private, the port is not.
    #[test]
    fn the_host_of_a_connecting_line_is_private() {
        let s = Scrubber::bare();
        assert_eq!(
            s.scrub("Connecting to play.friends-only.example, 25565"),
            "Connecting to <private>, 25565"
        );
        assert_eq!(
            s.scrub("[12:00:01] [Server Connector #1/INFO]: Connecting to mc.example.net, 25577\nnext line"),
            "[12:00:01] [Server Connector #1/INFO]: Connecting to <private>, 25577\nnext line"
        );
        // A host the other rules already replaced stays as they wrote it.
        assert_eq!(
            s.scrub("Connecting to 203.0.113.7, 25565"),
            "Connecting to <ip>, 25565"
        );
        // Only a whole placeholder stays: a host that merely starts with one is still private.
        assert_eq!(
            s.scrub("Connecting to <ip>secret.example, 25565"),
            "Connecting to <private>, 25565"
        );
        // The loopback short form is no address the IPv6 rule knows, so the host rule hides it.
        assert_eq!(
            s.scrub("Connecting to ::1, 25566"),
            "Connecting to <private>, 25566"
        );
        // Other text that starts the same way is not touched.
        assert_eq!(
            s.scrub("Connecting to the server..."),
            "Connecting to the server..."
        );
        assert_eq!(
            s.scrub("Connecting to the Realm, please wait"),
            "Connecting to the Realm, please wait"
        );
        // A sentence that happens to end in a comma and a number is not a connection record.
        assert_eq!(
            s.scrub("Connecting to the server failed on attempt, 3"),
            "Connecting to the server failed on attempt, 3"
        );
        // A known term that is the port (a save folder named 25565) must not hide the host's rule.
        assert_eq!(
            Scrubber::bare()
                .term("25565")
                .scrub("Connecting to play.example.net, 25565"),
            "Connecting to <private>, <private>"
        );
        // Two lines, one of them without a port.
        assert_eq!(
            s.scrub(
                "Connecting to a.example, 1\nConnecting to b.example\nConnecting to c.example, 2"
            ),
            "Connecting to <private>, 1\nConnecting to b.example\nConnecting to <private>, 2"
        );
    }

    /// The lowercased haystack and the text must share their character boundaries, not just their
    /// length: `İ` grows by a byte and U+212A shrinks by two, so a text holding two `İ` and a
    /// U+212A kept its length while its offsets moved, and a later match sliced mid-character.
    #[test]
    fn a_text_whose_lowercase_moves_its_boundaries_is_matched_as_it_is() {
        let s = Scrubber::bare().with_user("i\u{307}");
        assert_eq!(s.scrub("\u{130}\u{130}\u{212A}"), "\u{130}\u{130}\u{212A}");
        // A name that is there, in another case, is still hidden.
        assert_eq!(
            s.scrub("\u{130}\u{130}\u{212A} ANA"),
            "\u{130}\u{130}\u{212A} ANA"
        );
        assert_eq!(
            Scrubber::bare().with_user("ana").scrub("aNa here"),
            "<user> here"
        );
    }

    #[test]
    fn user_folders_lose_their_name() {
        let s = Scrubber::bare();
        assert_eq!(
            s.scrub("at /Users/ana.b/code/sunrise/src/main.rs:10"),
            "at /Users/<user>/code/sunrise/src/main.rs:10"
        );
        assert_eq!(
            s.scrub("/home/ana/.local/share/sunrise"),
            "/home/<user>/.local/share/sunrise"
        );
        assert_eq!(
            s.scrub("C:\\Users\\Ana Silva\\x\nnext"),
            "C:\\Users\\<user>\\x\nnext"
        );
        assert_eq!(s.scrub(r"C:\Users\ana\AppData"), r"C:\Users\<user>\AppData");
        assert_eq!(
            s.scrub(r#"{"path":"C:\\Users\\ana\\AppData"}"#),
            r#"{"path":"C:\\Users\\<user>\\AppData"}"#
        );
        assert_eq!(s.scrub("/usr/home/"), "/usr/home/");
    }

    #[test]
    fn the_user_name_goes_wherever_it_stands_alone() {
        let s = Scrubber::bare().with_user("ludo");
        assert_eq!(
            s.scrub("Ludo's machine, user ludo"),
            "<user>'s machine, user <user>"
        );
        assert_eq!(s.scrub("ludovic"), "ludovic");
        assert_eq!(Scrubber::bare().with_user("al").scrub("al"), "al");
        assert_eq!(
            Scrubber::bare()
                .with_user("user")
                .scrub(r"C:\Users\user\AppData, user"),
            r"C:\Users\<user>\AppData, <user>"
        );
    }

    #[test]
    fn addresses_and_accounts_are_hidden() {
        let s = Scrubber::bare();
        assert_eq!(s.scrub("joined 192.168.1.20:25565 ok"), "joined <ip> ok");
        assert_eq!(s.scrub("from 10.0.0.1."), "from <ip>.");
        assert_eq!(s.scrub("[2a01:e0a:1f2::5]"), "[<ip>]");
        assert_eq!(s.scrub("mail ana.b+mc@example.org now"), "mail <email> now");
        assert_eq!(
            s.scrub(
                "uuid 069a79f4-44e9-4726-a5be-fca90e38aaf5 and 069a79f444e94726a5befca90e38aaf5"
            ),
            "uuid <uuid> and <uuid>"
        );
    }

    #[test]
    fn code_and_versions_are_left_alone() {
        let s = Scrubber::bare();
        for text in [
            "std::panicking::begin_panic (src/game.rs:12:5)",
            "a::b::c",
            "driver 31.0.15.3623, Windows 10.0.19045.1",
            "wgpu 30.0.1",
            "commit 9fceb02d0ae598e95dc970b74767f19372d61af8",
            "user@host",
        ] {
            assert_eq!(s.scrub(text), text);
        }
    }

    #[test]
    fn known_private_terms_are_hidden() {
        let s = Scrubber::bare()
            .term("My World")
            .term("play.example.net")
            .term("ab");
        assert_eq!(
            s.scrub("Saving My World; joined play.example.net:25565, ab"),
            "Saving <private>; joined <private>:25565, ab"
        );
    }

    #[test]
    fn terms_match_any_case_and_word_terms_hide_short_names_safely() {
        let s = Scrubber::bare()
            .term("Private Profile")
            .term("Private")
            .term("Secret")
            .word_term("TV")
            .word_term("X")
            .word_term("ip");
        assert_eq!(
            s.scrub("There is already a profile called PRIVATE PROFILE and private profile."),
            "There is already a profile called <private> and <private>."
        );
        assert_eq!(
            s.scrub("TV uses 26.3-0.1.0; activity stays; Could not make profile x: denied; example stays"),
            "<private> uses 26.3-0.1.0; activity stays; Could not make profile <private>: denied; example stays"
        );
        assert_eq!(
            s.scrub("access_token=<secret> from 10.0.0.1 for Private Profile"),
            "access_token=<secret> from <ip> for <private>"
        );
        // A term containing `<` must still see an earlier enclosing placeholder.
        let angle = Scrubber::bare().term("ate>foo<b").term("Private Profile");
        assert_eq!(angle.scrub("Private Profilefoo<bar"), "<private>foo<bar");
    }

    #[test]
    fn path_roots_with_ips_and_aliases_are_hidden_before_ip_rewriting() {
        let unix = Scrubber::bare().path_term("/mnt/10.0.0.5/private/sunrise/");
        assert_eq!(
            unix.scrub("Could not open /mnt/10.0.0.5/private/sunrise/saves/World from 10.0.0.5"),
            "Could not open <private>/saves/World from <ip>"
        );
        // Absolute path passed through `term` gets the same before-IP treatment.
        assert_eq!(
            Scrubber::bare()
                .term("/mnt/10.0.0.5/private/sunrise")
                .scrub("open /mnt/10.0.0.5/private/sunrise/logs"),
            "open <private>/logs"
        );

        let drive = Scrubber::bare().path_term(r"D:\10.0.0.5\Private\Sunrise");
        for sample in [
            r"D:\10.0.0.5\Private\Sunrise\logs",
            r"d:\10.0.0.5\private\sunrise\logs",
            "D:/10.0.0.5/Private/Sunrise/logs",
            r"d:/10.0.0.5\Private/Sunrise/logs",
            r"\\?\D:\10.0.0.5\Private\Sunrise\logs",
            r"\\.\d:\10.0.0.5\private\sunrise\logs",
        ] {
            let scrubbed = drive.scrub(sample);
            assert!(
                scrubbed == r"<private>\logs" || scrubbed == "<private>/logs",
                "{sample} -> {scrubbed}"
            );
        }
        assert_eq!(
            drive.scrub(r#"{"path":"\\\\?\\d:\\10.0.0.5\\private\\sunrise\\logs"}"#),
            r#"{"path":"<private>\\logs"}"#
        );

        let unc = Scrubber::bare().path_term(r"\\10.0.0.5\Private\Sunrise");
        for sample in [
            r"\\10.0.0.5\Private\Sunrise\saves",
            r"\\?\UNC\10.0.0.5\private\sunrise\saves",
            "//10.0.0.5/Private/Sunrise/saves",
        ] {
            let scrubbed = unc.scrub(sample);
            assert!(
                scrubbed == r"<private>\saves" || scrubbed == "<private>/saves",
                "{sample} -> {scrubbed}"
            );
        }
        assert_eq!(
            unc.scrub(r#"{"unc":"\\\\?\\UNC\\10.0.0.5\\Private\\Sunrise\\saves"}"#),
            r#"{"unc":"<private>\\saves"}"#
        );

        let rel = Scrubber::bare().path_term("./private-dir/10.0.0.5/sunrise");
        assert_eq!(
            rel.scrub(
                "open ./private-dir/10.0.0.5/sunrise/saves and private-dir\\10.0.0.5\\sunrise\\logs"
            ),
            "open <private>/saves and <private>\\logs"
        );
        assert_eq!(
            rel.scrub("open ../private-dir/10.0.0.5/sunrise/saves"),
            "open ../<private>/saves"
        );
        // Standalone IPs and host rules remain intact.
        assert_eq!(
            unc.scrub("Connecting to 10.0.0.5, 25565"),
            "Connecting to <ip>, 25565"
        );
    }
}
