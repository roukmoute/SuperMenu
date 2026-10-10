use serde::{Deserialize, Serialize};
use std::cmp::Ordering;
use std::collections::{BinaryHeap, HashMap};
use std::time::Instant;
use unicode_normalization::{char::is_combining_mark, UnicodeNormalization};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Kind {
    Application,
    Install,
    Uninstall,
    File,
    Folder,
    Command,
    Web,
    Calculator,
    Timer,
    Clipboard,
    Symbol,
    Window,
    System,
    Navigation,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Item {
    pub id: String,
    pub title: String,
    pub subtitle: String,
    pub glyph: String,
    pub kind: Kind,
    pub target: String,
    pub arguments: String,
    #[serde(skip)]
    search_title: String,
    #[serde(skip)]
    search_subtitle: String,
    #[serde(skip)]
    title_mask: u64,
    #[serde(skip)]
    subtitle_mask: u64,
}

impl Item {
    pub fn new(
        id: impl Into<String>,
        title: impl Into<String>,
        subtitle: impl Into<String>,
        glyph: impl Into<String>,
        kind: Kind,
        target: impl Into<String>,
    ) -> Self {
        let title = title.into();
        let subtitle = subtitle.into();
        let search_title = normalize(&title);
        let search_subtitle = normalize(&subtitle);
        Self {
            id: id.into(),
            title_mask: character_mask(&search_title),
            subtitle_mask: character_mask(&search_subtitle),
            search_title,
            search_subtitle,
            title,
            subtitle,
            glyph: glyph.into(),
            kind,
            target: target.into(),
            arguments: String::new(),
        }
    }

    /// Refresh caches after deserialization or an explicit title/subtitle edit.
    pub fn prepare(&mut self) {
        self.search_title = normalize(&self.title);
        self.search_subtitle = normalize(&self.subtitle);
        self.title_mask = character_mask(&self.search_title);
        self.subtitle_mask = character_mask(&self.search_subtitle);
    }
}

fn normalize(text: &str) -> String {
    let mut result = String::with_capacity(text.len());
    let mut space = true;
    for character in text
        .nfd()
        .filter(|character| !is_combining_mark(*character))
    {
        if character.is_alphanumeric() {
            for lowercase in character.to_lowercase() {
                match lowercase {
                    'œ' => result.push_str("oe"),
                    'æ' => result.push_str("ae"),
                    'ß' => result.push_str("ss"),
                    'ø' => result.push('o'),
                    other => result.push(other),
                }
            }
            space = false;
        } else if !space {
            result.push(' ');
            space = true;
        }
    }
    if result.ends_with(' ') {
        result.pop();
    }
    result
}

/// Selects top K without sorting the full index. Only selected items are cloned.
pub fn rank(items: &[Item], query: &str, limit: usize, usage: &HashMap<String, u32>) -> Vec<Item> {
    rank_filtered(items, query, limit, usage, |_| true)
}

/// Filters in place before ranking, keeping the index borrowed and avoiding full-index clones.
pub fn rank_filtered(
    items: &[Item],
    query: &str,
    limit: usize,
    usage: &HashMap<String, u32>,
    accept: impl Fn(&Item) -> bool,
) -> Vec<Item> {
    if limit == 0 {
        return Vec::new();
    }
    let limit = limit.min(1000);
    let query = normalize(query);
    if query.len() > 512 {
        return Vec::new();
    }
    let tokens: Vec<Token<'_>> = query
        .split_whitespace()
        .map(|text| Token {
            text,
            mask: character_mask(text),
        })
        .collect();
    let mut heap: BinaryHeap<Candidate<'_>> = BinaryHeap::with_capacity(limit);
    for (ordinal, item) in items.iter().enumerate() {
        if !accept(item) {
            continue;
        }
        let Some(mut score) = score(item, &query, &tokens) else {
            continue;
        };
        if let Some(count) = usage.get(&item.id).filter(|count| **count > 0) {
            score += (count.saturating_add(1).ilog2() * 18).min(150) as i32;
        }
        let candidate = Candidate {
            score,
            item,
            ordinal,
        };
        if heap.len() < limit {
            heap.push(candidate);
        } else if heap.peek().is_some_and(|worst| candidate < *worst) {
            *heap.peek_mut().expect("nonempty heap") = candidate;
        }
    }
    heap.into_sorted_vec()
        .into_iter()
        .map(|candidate| candidate.item.clone())
        .collect()
}

struct Token<'a> {
    text: &'a str,
    mask: u64,
}

fn character_mask(text: &str) -> u64 {
    let mut mask = 0;
    for byte in text.bytes() {
        let bit = match byte {
            b'a'..=b'z' => byte - b'a',
            b'0'..=b'9' => byte - b'0' + 26,
            _ => continue,
        };
        mask |= 1_u64 << bit;
    }
    mask
}

fn score(item: &Item, query: &str, tokens: &[Token<'_>]) -> Option<i32> {
    if tokens.is_empty() {
        return Some(0);
    }
    let mut score = 0;
    for token in tokens {
        let title = if item.title_mask & token.mask == token.mask {
            match_token(&item.search_title, token.text).map(|score| score * 4)
        } else {
            None
        };
        // A subtitle cannot score over 250, so do not scan it when the title already wins.
        if title.is_some_and(|score| score >= 250) {
            score += title?;
            continue;
        }
        let subtitle = if item.subtitle_mask & token.mask == token.mask {
            match_token(&item.search_subtitle, token.text)
        } else {
            None
        };
        score += match (title, subtitle) {
            (Some(title), Some(subtitle)) => title.max(subtitle),
            (Some(title), None) => title,
            (None, Some(subtitle)) => subtitle,
            (None, None) => return None,
        };
    }
    score += if item.search_title == query {
        1000
    } else if item.search_title.starts_with(query) {
        550
    } else if item.search_title.contains(query) {
        200
    } else {
        0
    };
    Some(score)
}

fn match_token(source: &str, token: &str) -> Option<i32> {
    if source.is_empty() || token.len() > source.len() {
        return None;
    }
    if let Some(position) = source.find(token) {
        let boundary = position == 0 || source.as_bytes()[position - 1] == b' ';
        let end = position + token.len();
        let end_boundary = end == source.len() || source.as_bytes()[end] == b' ';
        return Some(
            160 + i32::from(boundary) * 55
                + i32::from(end_boundary) * 20
                + i32::from(position == 0) * 15,
        );
    }
    // Ignore word separators, never intervening letters: Firefox still matches
    // "Fire Fox", but Steam cannot match "Stream" by silently dropping its r.
    if let Some(score) = match_joined_words(source, token) {
        return Some(score);
    }
    if let Some(score) = match_initials(source, token) {
        return Some(score);
    }
    // Short, word-anchored contractions remain useful (ffx -> Firefox). Longer
    // contractions need an explicit alias rather than arbitrary subsequences.
    if token.is_ascii() {
        return match_ascii_abbreviation(source.as_bytes(), token.as_bytes());
    }
    let length = token.chars().count();
    if !(2..=3).contains(&length) {
        return None;
    }
    let mut best = None;
    for word in source.split_whitespace() {
        if word.chars().next() != token.chars().next() {
            continue;
        }
        let mut sought = token.chars();
        let mut next = sought.next()?;
        let mut last = None;
        let mut consecutive = 0;
        let mut matched = 0;
        for (position, character) in word.chars().take(length * 3).enumerate() {
            if character != next {
                continue;
            }
            if last.is_some_and(|last| position == last + 1) {
                consecutive += 1;
            }
            last = Some(position);
            matched += 1;
            if let Some(character) = sought.next() {
                next = character;
            } else {
                let gaps = position + 1 - matched;
                let score = 91 + consecutive * 9 - gaps as i32 * 3;
                best = Some(best.map_or(score, |previous: i32| previous.max(score)));
                break;
            }
        }
    }
    best
}

fn match_joined_words(source: &str, token: &str) -> Option<i32> {
    if !source.as_bytes().contains(&b' ') {
        return None;
    }
    let first = token.chars().next()?;
    for (start, character) in source.char_indices() {
        if character != first {
            continue;
        }
        let mut candidate = source[start..].char_indices().filter(|(_, c)| *c != ' ');
        let mut end = start;
        let mut matched = true;
        for sought in token.chars() {
            match candidate.next() {
                Some((position, character)) if character == sought => {
                    end = start + position + character.len_utf8();
                }
                _ => {
                    matched = false;
                    break;
                }
            }
        }
        if matched {
            let boundary = start == 0 || source.as_bytes()[start - 1] == b' ';
            let end_boundary = end == source.len() || source.as_bytes()[end] == b' ';
            return Some(
                145 + i32::from(boundary) * 55
                    + i32::from(end_boundary) * 20
                    + i32::from(start == 0) * 15,
            );
        }
    }
    None
}

fn match_initials(source: &str, token: &str) -> Option<i32> {
    let first = token.chars().next()?;
    let mut words = source.split_whitespace();
    while let Some(word) = words.next() {
        if !word.starts_with(first) {
            continue;
        }
        let mut candidate = words.clone();
        if token.chars().skip(1).all(|character| {
            candidate.next().and_then(|word| word.chars().next()) == Some(character)
        }) {
            return Some(110);
        }
    }
    None
}

fn match_ascii_abbreviation(source: &[u8], token: &[u8]) -> Option<i32> {
    if !(2..=3).contains(&token.len()) {
        return None;
    }
    let mut best = None;
    for word in source.split(|byte| *byte == b' ') {
        if word.first() != token.first() {
            continue;
        }
        let mut sought = 0;
        let mut last = usize::MAX;
        let mut consecutive = 0;
        for (position, &byte) in word.iter().take(token.len() * 3).enumerate() {
            if byte != token[sought] {
                continue;
            }
            if last != usize::MAX && position == last + 1 {
                consecutive += 1;
            }
            last = position;
            sought += 1;
            if sought == token.len() {
                let gaps = position + 1 - token.len();
                let score = 91 + consecutive * 9 - gaps as i32 * 3;
                best = Some(best.map_or(score, |previous: i32| previous.max(score)));
                break;
            }
        }
    }
    best
}

#[derive(Debug)]
struct Candidate<'a> {
    score: i32,
    item: &'a Item,
    ordinal: usize,
}

impl PartialEq for Candidate<'_> {
    fn eq(&self, other: &Self) -> bool {
        self.cmp(other) == Ordering::Equal
    }
}
impl Eq for Candidate<'_> {}
impl PartialOrd for Candidate<'_> {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}
impl Ord for Candidate<'_> {
    fn cmp(&self, other: &Self) -> Ordering {
        // Largest = worst, making the heap's root the candidate to replace.
        other
            .score
            .cmp(&self.score)
            .then_with(|| self.item.search_title.cmp(&other.item.search_title))
            .then_with(|| self.item.id.cmp(&other.item.id))
            .then_with(|| self.ordinal.cmp(&other.ordinal))
    }
}

/// A bounded arithmetic parser. Input is never passed to a shell or script engine.
pub fn calculate(expression: &str) -> Option<String> {
    if expression.trim().is_empty() || expression.len() > 512 {
        return None;
    }
    let mut parser = Parser {
        source: expression.as_bytes(),
        position: 0,
        depth: 0,
        operations: 0,
    };
    let value = parser.sum().ok()?;
    parser.whitespace();
    if parser.position != parser.source.len() || !value.is_finite() {
        return None;
    }
    let value = if value == 0.0 { 0.0 } else { value };
    Some(
        if value != 0.0 && (value.abs() >= 1e16 || value.abs() < 1e-8) {
            format!("{value:e}")
        } else {
            value.to_string()
        },
    )
}

type ParseResult = Result<f64, ()>;
struct Parser<'a> {
    source: &'a [u8],
    position: usize,
    depth: usize,
    operations: usize,
}

impl Parser<'_> {
    fn sum(&mut self) -> ParseResult {
        let mut value = self.product()?;
        loop {
            if self.take(b'+') {
                let rhs = self.product()?;
                value = self.finite(value + rhs)?;
            } else if self.take(b'-') {
                let rhs = self.product()?;
                value = self.finite(value - rhs)?;
            } else {
                return Ok(value);
            }
        }
    }
    fn product(&mut self) -> ParseResult {
        let mut value = self.unary()?;
        loop {
            if self.take(b'*') {
                let rhs = self.unary()?;
                value = self.finite(value * rhs)?;
            } else if self.take(b'/') {
                let rhs = self.unary()?;
                if rhs == 0.0 {
                    return Err(());
                }
                value = self.finite(value / rhs)?;
            } else if self.take(b'%') {
                let rhs = self.unary()?;
                if rhs == 0.0 {
                    return Err(());
                }
                value = self.finite(value % rhs)?;
            } else {
                return Ok(value);
            }
        }
    }
    fn unary(&mut self) -> ParseResult {
        self.depth += 1;
        if self.depth > 32 {
            return Err(());
        }
        let result = if self.take(b'+') {
            self.unary()
        } else if self.take(b'-') {
            let value = self.unary()?;
            self.finite(-value)
        } else {
            self.power()
        };
        self.depth -= 1;
        result
    }
    fn power(&mut self) -> ParseResult {
        let value = self.primary()?;
        if self.take(b'^') {
            let rhs = self.unary()?;
            self.finite(value.powf(rhs))
        } else {
            Ok(value)
        }
    }
    fn primary(&mut self) -> ParseResult {
        self.whitespace();
        if self.take(b'(') {
            let value = self.sum()?;
            self.require(b')')?;
            return Ok(value);
        }
        if self
            .source
            .get(self.position..self.position + 2)
            .is_some_and(|s| s == "π".as_bytes())
        {
            self.position += 2;
            return Ok(std::f64::consts::PI);
        }
        if self
            .source
            .get(self.position)
            .is_some_and(u8::is_ascii_alphabetic)
        {
            let start = self.position;
            self.position += 1;
            while self
                .source
                .get(self.position)
                .is_some_and(u8::is_ascii_alphanumeric)
            {
                self.position += 1;
            }
            let name = std::str::from_utf8(&self.source[start..self.position])
                .map_err(|_| ())?
                .to_ascii_lowercase();
            if name == "pi" {
                return Ok(std::f64::consts::PI);
            }
            if name == "e" {
                return Ok(std::f64::consts::E);
            }
            self.require(b'(')?;
            let first = self.sum()?;
            let second = if self.take(b',') {
                Some(self.sum()?)
            } else {
                None
            };
            self.require(b')')?;
            return self.function(&name, first, second);
        }
        self.number()
    }
    fn number(&mut self) -> ParseResult {
        let start = self.position;
        let mut digit = false;
        while self
            .source
            .get(self.position)
            .is_some_and(u8::is_ascii_digit)
        {
            self.position += 1;
            digit = true;
        }
        if self.source.get(self.position) == Some(&b'.') {
            self.position += 1;
            while self
                .source
                .get(self.position)
                .is_some_and(u8::is_ascii_digit)
            {
                self.position += 1;
                digit = true;
            }
        }
        if !digit {
            return Err(());
        }
        if self
            .source
            .get(self.position)
            .is_some_and(|c| matches!(c, b'e' | b'E'))
        {
            self.position += 1;
            if self
                .source
                .get(self.position)
                .is_some_and(|c| matches!(c, b'+' | b'-'))
            {
                self.position += 1;
            }
            let exponent = self.position;
            while self
                .source
                .get(self.position)
                .is_some_and(u8::is_ascii_digit)
            {
                self.position += 1;
            }
            if exponent == self.position {
                return Err(());
            }
        }
        let value = std::str::from_utf8(&self.source[start..self.position])
            .map_err(|_| ())?
            .parse::<f64>()
            .map_err(|_| ())?;
        self.finite(value)
    }
    fn function(&mut self, name: &str, first: f64, second: Option<f64>) -> ParseResult {
        let value = if let Some(other) = second {
            match name {
                "pow" => first.powf(other),
                "min" => first.min(other),
                "max" => first.max(other),
                "log" if first > 0.0 && other > 0.0 && other != 1.0 => first.log(other),
                "round" if other == other.trunc() && (0.0..=12.0).contains(&other) => {
                    let factor = 10_f64.powi(other as i32);
                    (first * factor).round() / factor
                }
                _ => return Err(()),
            }
        } else {
            match name {
                "sqrt" if first >= 0.0 => first.sqrt(),
                "sin" => first.sin(),
                "cos" => first.cos(),
                "tan" => first.tan(),
                "log" | "ln" if first > 0.0 => first.ln(),
                "log10" if first > 0.0 => first.log10(),
                "exp" => first.exp(),
                "abs" => first.abs(),
                "round" => first.round(),
                "floor" => first.floor(),
                "ceil" => first.ceil(),
                _ => return Err(()),
            }
        };
        self.finite(value)
    }
    fn finite(&mut self, value: f64) -> ParseResult {
        self.operations += 1;
        if self.operations > 256 || !value.is_finite() {
            Err(())
        } else {
            Ok(value)
        }
    }
    fn take(&mut self, token: u8) -> bool {
        self.whitespace();
        if self.source.get(self.position) == Some(&token) {
            self.position += 1;
            true
        } else {
            false
        }
    }
    fn require(&mut self, token: u8) -> Result<(), ()> {
        if self.take(token) {
            Ok(())
        } else {
            Err(())
        }
    }
    fn whitespace(&mut self) {
        while self
            .source
            .get(self.position)
            .is_some_and(u8::is_ascii_whitespace)
        {
            self.position += 1;
        }
    }
}

pub fn benchmark() -> String {
    let names = [
        "Visual Studio Code",
        "Firefox",
        "Calculatrice",
        "Terminal Windows",
        "Résumé réunion",
        "Explorateur",
        "PowerShell",
        "Microsoft Word",
    ];
    let items: Vec<Item> = (0..20_000)
        .map(|index| {
            Item::new(
                format!("item{index:05}"),
                format!("{} {index:05}", names[index % names.len()]),
                format!("Documents / Projet {} / Note {index:05}", index % 97),
                "□",
                Kind::Application,
                "",
            )
        })
        .collect();
    let queries = [
        "vsc",
        "firefox",
        "resume",
        "terminal windows",
        "note 012",
        "qqqqq",
        "",
        "power",
    ];
    let files = [
        "Résumé réunion.docx",
        "Budget annuel.xlsx",
        "Photo vacances.jpg",
        "Rapport projet.pdf",
        "Notes techniques.txt",
        "Archive backup.zip",
    ];
    let mixed: Vec<Item> = (0..20_000)
        .map(|index| {
            let (title, kind) = if index < 1000 {
                (
                    format!("{} {index:05}", names[index % names.len()]),
                    Kind::Application,
                )
            } else if index % 9 == 0 {
                (format!("Projet {index:05}"), Kind::Folder)
            } else {
                (
                    format!("{index:05} {}", files[index % files.len()]),
                    Kind::File,
                )
            };
            Item::new(
                format!("mixed{index:05}"),
                title,
                format!(
                    "C:\\Users\\User\\Documents\\Projet {}\\{index:05}",
                    index % 97
                ),
                "□",
                kind,
                "",
            )
        })
        .collect();
    let mixed_queries = [
        "budget",
        "resume reunion",
        "photo",
        "projet 012",
        "notes",
        "backup",
        "",
        "qqqqq",
    ];
    let usage = HashMap::new();
    let measure = |index_items: &[Item], queries: &[&str], files_only: bool| {
        let search = |index| {
            rank_filtered(
                index_items,
                queries[index % queries.len()],
                9,
                &usage,
                |item| !files_only || matches!(item.kind, Kind::File | Kind::Folder),
            )
        };
        for index in 0..100 {
            std::hint::black_box(search(index));
        }
        let mut samples = Vec::with_capacity(400);
        for index in 0..400 {
            let started = Instant::now();
            std::hint::black_box(search(index));
            samples.push(started.elapsed().as_secs_f64() * 1000.0);
        }
        samples.sort_by(f64::total_cmp);
        (samples[200], samples[379])
    };
    let simple = measure(&items, &queries, false);
    let mixed_stats = measure(&mixed, &mixed_queries, true);
    let missing = crate::packages::missing_items(&mixed, &[], &Default::default());
    let removals = crate::packages::removable_items(&mixed, &[]);
    let search = |ordinal: usize| {
        crate::packages::search_with_removals(
            &mixed,
            &missing,
            &removals,
            queries[ordinal % queries.len()],
            9,
            &usage,
        )
    };
    for ordinal in 0..100 {
        std::hint::black_box(search(ordinal));
    }
    let mut samples = Vec::with_capacity(400);
    for ordinal in 0..400 {
        let started = Instant::now();
        std::hint::black_box(search(ordinal));
        samples.push(started.elapsed().as_secs_f64() * 1000.0);
    }
    samples.sort_by(f64::total_cmp);
    format!("20,000 indexed items, top 9, 100 warmups, 400 samples per scenario\nSimple index median: {:.3} ms; p95: {:.3} ms.\nMixed index /files median: {:.3} ms; p95: {:.3} ms.\nMixed index apps + cached package actions median: {:.3} ms; p95: {:.3} ms.\nIn-memory ranking only; indexing, activation and cold startup are separate.",simple.0,simple.1,mixed_stats.0,mixed_stats.1,samples[200],samples[379])
}

#[cfg(test)]
mod tests {
    use super::*;
    fn item(id: &str, title: &str, subtitle: &str) -> Item {
        Item::new(id, title, subtitle, "□", Kind::Application, "")
    }
    fn number(expression: &str, expected: f64) {
        let actual = calculate(expression)
            .unwrap_or_else(|| panic!("Rejected {expression}"))
            .parse::<f64>()
            .unwrap();
        assert!(
            (actual - expected).abs() <= expected.abs().max(1.0) * 1e-12,
            "{expression}: got {actual}, expected {expected}"
        );
    }
    #[test]
    fn calculator_precedence_and_constants() {
        for (expression, expected) in [
            ("2+3*4", 14.0),
            ("(2+3)*4", 20.0),
            ("2^3^2", 512.0),
            ("-2^2", -4.0),
            ("(-2)^2", 4.0),
            ("2^-2", 0.25),
            ("10%3", 1.0),
            ("1.2e3+.5", 1200.5),
            ("2*pi", std::f64::consts::PI * 2.0),
            ("e", std::f64::consts::E),
            ("π", std::f64::consts::PI),
            ("10/2/5", 1.0),
            ("10-2-3", 5.0),
            ("--2", 2.0),
        ] {
            number(expression, expected);
        }
        assert_eq!(calculate("-0"), Some("0".to_string()));
    }
    #[test]
    fn calculator_functions_and_numeric_format() {
        for (expression, expected) in [
            ("sqrt(81)", 9.0),
            ("sin(pi/2)", 1.0),
            ("cos(0)", 1.0),
            ("log(e)", 1.0),
            ("log(100,10)", 2.0),
            ("abs(-3.5)", 3.5),
            ("round(2.345,2)", 2.35),
            ("round(-2.5)", -3.0),
            ("pow(2,8)", 256.0),
            ("max(4,min(8,3))", 4.0),
            ("log10(1000)", 3.0),
            ("sqrt(abs(-16))", 4.0),
            ("1e-20", 1e-20),
        ] {
            number(expression, expected);
        }
        assert_eq!(calculate("1.25+.5"), Some("1.75".to_string()));
    }
    #[test]
    fn calculator_rejects_invalid_and_excessive_input() {
        for expression in [
            "",
            " ",
            "2+",
            "(2",
            "2)",
            "2**3",
            "1/0",
            "0/0",
            "10%0",
            "sqrt(-1)",
            "log(0)",
            "log(2,1)",
            "pow(-1,.5)",
            "exp(10000)",
            "1e400",
            "1e+",
            ".",
            "2pi",
            "foo(1)",
            "sin(1,2)",
            "round(1,13)",
            "round(1,1.5)",
            "1,5",
            "Process.Start(1)",
            "1;2",
            "min(1,2,3)",
        ] {
            assert!(calculate(expression).is_none(), "Accepted {expression}");
        }
        for expression in [
            "(".repeat(40) + "1" + &")".repeat(40),
            "-".repeat(40) + "1",
            "1".repeat(513),
            vec!["1"; 180].join("+"),
        ] {
            assert!(calculate(&expression).is_none());
        }
    }
    #[test]
    fn search_accents_tokens_and_abbreviations() {
        let items = [
            item("calc", "Calculatrice", "Outil système"),
            item("code", "Visual Studio Code", "Éditeur de code"),
            item("notes", "Résumé réunion", "Documents / Projet étoilé"),
            item("browser", "Firefox", "Navigateur web"),
        ];
        let usage = HashMap::new();
        for (query, expected) in [
            ("RESUME", "notes"),
            ("studio editeur", "code"),
            ("vsc", "code"),
            ("ffx", "browser"),
            ("  résumé   réunion ", "notes"),
        ] {
            assert_eq!(rank(&items, query, 9, &usage)[0].id, expected);
        }
        assert!(rank(&items, "nomatch qqqq", 9, &usage).is_empty());
        assert!(rank(&items, &"a".repeat(513), 9, &usage).is_empty());
    }
    #[test]
    fn search_title_relevance_and_deterministic_ties() {
        let items = [
            item("subtitle", "Other tool", "Firefox"),
            item("partial", "Firefox Developer", ""),
            item("exact", "Firefox", ""),
            item("fuzzy", "Fire flux", ""),
        ];
        let usage = HashMap::new();
        let hits = rank(&items, "firefox", 2, &usage);
        assert_eq!(
            hits.iter().map(|item| item.id.as_str()).collect::<Vec<_>>(),
            ["exact", "partial"]
        );
        let ties = [
            item("b", "Beta", ""),
            item("a2", "Alpha", ""),
            item("a1", "Alpha", ""),
        ];
        assert_eq!(
            rank(&ties, "", 2, &usage)
                .iter()
                .map(|item| item.id.as_str())
                .collect::<Vec<_>>(),
            ["a1", "a2"]
        );
        let reversed = ties.iter().rev().cloned().collect::<Vec<_>>();
        assert_eq!(
            rank(&reversed, "", 2, &usage)
                .iter()
                .map(|item| item.id.as_str())
                .collect::<Vec<_>>(),
            ["a1", "a2"]
        );
        assert_eq!(
            rank(&ties, "", 1, &HashMap::from([("b".to_string(), 15)]))[0].id,
            "b"
        );
        assert!(rank(&ties, "", 0, &usage).is_empty());
    }
    #[test]
    fn full_application_names_reject_scattered_letters_and_action_descriptions() {
        let mut telegram = item(
            "telegram-action",
            "Telegram",
            "Désinstaller et nettoyer · Messagerie",
        );
        telegram.kind = Kind::Uninstall;
        let items = [
            item("steam", "Steam", "Application Windows"),
            item("steam-client", "Steam Client", "Jeux"),
            item("stream", "Stream Deck", "Application Windows"),
            item("gamespy", "Register GameSpy Arcade", "Application Windows"),
            item("teams", "Microsoft Teams", "Application Windows"),
            item(
                "telegram-title",
                "Désinstaller Telegram",
                "Application Windows",
            ),
            telegram,
        ];
        let usage = items
            .iter()
            .map(|item| (item.id.clone(), u32::MAX))
            .collect();
        let hits = rank(&items, "steam", 20, &usage);
        assert_eq!(
            hits.iter().map(|item| item.id.as_str()).collect::<Vec<_>>(),
            ["steam", "steam-client"]
        );
        assert_eq!(rank(&items, "teams", 20, &usage)[0].id, "teams");
        assert!(rank(&items, "telgram", 20, &usage).is_empty());
    }
    #[test]
    fn word_initials_and_compact_names_preserve_useful_abbreviations() {
        let items = [
            item("code", "Microsoft Visual Studio Code", "Éditeur"),
            item("terminal", "Windows Terminal", "Console"),
            item("browser", "Firefox", "Navigateur"),
            item("split-browser", "Fire Fox Toolkit", "Navigateur"),
            item("initials", "Alpha Alpha Beta Code", ""),
            item("long-initials", "Windows Remote Desktop Connection", ""),
        ];
        for (query, expected) in [
            ("vsc", "code"),
            ("mvsc", "code"),
            ("wt", "terminal"),
            ("ffx", "browser"),
            ("abc", "initials"),
            ("wrdc", "long-initials"),
        ] {
            assert_eq!(
                rank(&items, query, 9, &HashMap::new())[0].id,
                expected,
                "{query}"
            );
        }
        let browsers = rank(&items, "firefox", 9, &HashMap::new());
        assert_eq!(
            browsers
                .iter()
                .map(|item| item.id.as_str())
                .collect::<Vec<_>>(),
            ["browser", "split-browser"]
        );
    }
    #[test]
    fn short_contractions_are_word_anchored_and_bounded() {
        let items = [
            item("browser", "Firefox", ""),
            item("inside-word", "Xfirefox", ""),
            item("scattered", "Faaaaaaaafaaaaaaaax", ""),
            item("cross-word", "Fire Fantastic Example", ""),
        ];
        assert_eq!(
            rank(&items, "ffx", 9, &HashMap::new())
                .iter()
                .map(|item| item.id.as_str())
                .collect::<Vec<_>>(),
            ["browser"]
        );
        assert!(rank(&items, "frfx", 9, &HashMap::new()).is_empty());
        // Initials are consecutive words, not arbitrary letters or word skips.
        assert!(rank(
            &[item("initials", "Visual Other Studio Code", "")],
            "vsc",
            9,
            &HashMap::new()
        )
        .is_empty());
    }
    #[test]
    fn search_top_k_agrees_with_full_ordering() {
        let mut items: Vec<Item> = (0..2000)
            .map(|index| {
                item(
                    &format!("id{index:04}"),
                    &format!("App {:04}", 1999 - index),
                    "",
                )
            })
            .collect();
        let selected = rank(&items, "", 9, &HashMap::new());
        items.sort_by(|a, b| a.title.cmp(&b.title).then_with(|| a.id.cmp(&b.id)));
        assert_eq!(
            selected.iter().map(|item| &item.id).collect::<Vec<_>>(),
            items[..9].iter().map(|item| &item.id).collect::<Vec<_>>()
        );
    }
    #[test]
    fn search_filter_borrows_index_and_cache_can_be_restored() {
        let app = item("app", "Résumé app", "Éditeur");
        let mut file = Item::new("file", "Résumé.docx", "Documents", "□", Kind::File, "");
        let serialized = serde_json::to_string(&file).unwrap();
        assert!(!serialized.contains("search_title") && !serialized.contains("title_mask"));
        file = serde_json::from_str(&serialized).unwrap();
        file.prepare();
        let hits = rank_filtered(&[app, file], "resume", 9, &HashMap::new(), |item| {
            item.kind == Kind::File
        });
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].id, "file");
        let mut changed = item("renamed", "Before", "");
        changed.title = "Après".into();
        changed.prepare();
        assert_eq!(rank(&[changed], "apres", 9, &HashMap::new()).len(), 1);
    }
    #[test]
    fn search_non_ascii_and_ligatures_remain_searchable() {
        let items = [
            item("ru", "Редактор документов", ""),
            item("ligature", "Cœur Straße", ""),
        ];
        let usage = HashMap::new();
        assert_eq!(rank(&items, "ркт", 9, &usage)[0].id, "ru");
        assert_eq!(rank(&items, "редактор", 9, &usage)[0].id, "ru");
        assert!(rank(&items, "рктр", 9, &usage).is_empty());
        assert_eq!(rank(&items, "coeur strasse", 9, &usage)[0].id, "ligature");
    }
}
