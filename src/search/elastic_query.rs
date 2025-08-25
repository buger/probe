use std::collections::HashMap;
use std::collections::HashSet;
use std::hash::{Hash, Hasher};
use std::iter::Peekable;
use std::str::Chars;

// PHASE 3C OPTIMIZATION: Compute hash key for evaluation cache
fn compute_evaluation_key(matched_terms: &HashSet<usize>) -> u64 {
    use std::collections::hash_map::DefaultHasher;
    let mut hasher = DefaultHasher::new();
    let mut sorted_terms: Vec<_> = matched_terms.iter().cloned().collect();
    sorted_terms.sort_unstable();
    sorted_terms.hash(&mut hasher);
    hasher.finish()
}

/// The AST representing a parsed query.
#[derive(Debug, Clone, PartialEq)]
pub enum Expr {
    /// A search term, which can represent multiple keywords.
    /// `keywords` => a list of keywords (possibly tokenized/split)
    /// `field` => optional field specifier (e.g. `Some("title")` for `title:foo`)
    /// `required` => a leading `+`
    /// `excluded` => a leading `-`
    /// `exact` => if originally quoted, meaning "no tokenization/splitting"
    Term {
        keywords: Vec<String>,
        field: Option<String>,
        required: bool,
        excluded: bool,
        exact: bool,
    },

    /// Logical AND of two sub-expressions.
    And(Box<Expr>, Box<Expr>),

    /// Logical OR of two sub-expressions.
    Or(Box<Expr>, Box<Expr>),
}

impl Expr {
    /// Extract required and optional terms from the AST, excluding negative terms
    #[cfg(test)]
    pub fn extract_terms(&self) -> (Vec<String>, Vec<String>) {
        let mut required = Vec::new();
        let mut optional = Vec::new();
        self.collect_terms(&mut required, &mut optional);
        (required, optional)
    }

    #[cfg(test)]
    fn collect_terms(&self, required: &mut Vec<String>, optional: &mut Vec<String>) {
        match self {
            Expr::Term {
                keywords,
                required: is_required,
                excluded,
                ..
            } => {
                if !excluded {
                    for keyword in keywords {
                        if *is_required {
                            required.push(keyword.clone());
                        } else {
                            optional.push(keyword.clone());
                        }
                    }
                }
            }
            Expr::And(left, right) => {
                left.collect_terms(required, optional);
                right.collect_terms(required, optional);
            }
            Expr::Or(left, right) => {
                left.collect_terms(required, optional);
                right.collect_terms(required, optional);
            }
        }
    }

    /// Returns `true` if this expression contains at least one `required=true` term.
    pub fn has_required_term(&self) -> bool {
        match self {
            Expr::Term { required, .. } => *required,
            Expr::And(left, right) | Expr::Or(left, right) => {
                left.has_required_term() || right.has_required_term()
            }
        }
    }

    /// Returns `true` if this expression contains only excluded terms.
    /// This is used for early termination optimization.
    pub fn is_only_excluded_terms(&self) -> bool {
        match self {
            Expr::Term { excluded, .. } => *excluded,
            Expr::And(left, right) => {
                left.is_only_excluded_terms() && right.is_only_excluded_terms()
            }
            Expr::Or(left, right) => {
                left.is_only_excluded_terms() && right.is_only_excluded_terms()
            }
        }
    }

    /// Check if all required terms in the expression are present in matched_terms
    /// This is the critical fix for Lucene semantics
    fn check_all_required_terms_present(
        &self,
        matched_terms: &HashSet<usize>,
        term_indices: &HashMap<String, usize>,
    ) -> bool {
        match self {
            Expr::Term {
                keywords,
                required,
                excluded,
                ..
            } => {
                if *required && !*excluded {
                    // All keywords in this required term must be present
                    keywords.iter().all(|kw| {
                        term_indices
                            .get(kw)
                            .map(|idx| matched_terms.contains(idx))
                            .unwrap_or(false)
                    })
                } else {
                    // Not a required term, so it doesn't affect required term checking
                    true
                }
            }
            Expr::And(left, right) => {
                // For AND: both sides must have their required terms satisfied
                left.check_all_required_terms_present(matched_terms, term_indices)
                    && right.check_all_required_terms_present(matched_terms, term_indices)
            }
            Expr::Or(left, right) => {
                // For OR: both sides must have their required terms satisfied
                // This is crucial - even in OR, required terms must be present
                left.check_all_required_terms_present(matched_terms, term_indices)
                    && right.check_all_required_terms_present(matched_terms, term_indices)
            }
        }
    }

    /// A helper to evaluate the expression when the caller already knows if
    /// there are any required terms in the *entire* query (not just in this subtree).
    fn evaluate_with_has_required(
        &self,
        matched_terms: &HashSet<usize>,
        term_indices: &HashMap<String, usize>,
        ignore_negatives: bool,
        has_required_anywhere: bool,
    ) -> bool {
        // Early termination optimization: if no terms matched, result is always false
        // (unless we have only excluded terms, but that's handled below)
        if matched_terms.is_empty() && !self.is_only_excluded_terms() {
            return false;
        }

        let debug_mode = std::env::var("PROBE_DEBUG").unwrap_or_default() == "1";

        // CRITICAL FIX: Check required terms FIRST before any other evaluation
        // In Lucene semantics, if ANY required term is missing, the entire query fails
        if has_required_anywhere && !ignore_negatives {
            let all_required_terms_present =
                self.check_all_required_terms_present(matched_terms, term_indices);
            if !all_required_terms_present {
                if debug_mode {
                    println!("DEBUG: Query failed - required terms missing");
                }
                return false;
            }
        }

        match self {
            Expr::Term {
                keywords,
                required,
                excluded,
                ..
            } => {
                if keywords.is_empty() {
                    // Empty term => if excluded, trivially true, otherwise false
                    return *excluded;
                }
                // Are all keywords present?
                let all_present = keywords.iter().all(|kw| {
                    term_indices
                        .get(kw)
                        .map(|idx| matched_terms.contains(idx))
                        .unwrap_or(false)
                });

                if *excluded {
                    if ignore_negatives {
                        // Negative ignored => always true
                        true
                    } else {
                        // Excluded => none should be present
                        !keywords.iter().any(|kw| {
                            term_indices
                                .get(kw)
                                .map(|idx| matched_terms.contains(idx))
                                .unwrap_or(false)
                        })
                    }
                } else if *required && ignore_negatives {
                    // If ignoring negatives, we've already enforced required terms up front.
                    true
                } else if *required {
                    // Required => must all be present
                    all_present
                } else {
                    // Optional => if there's at least one required term anywhere in the entire query,
                    // then we do NOT fail if this optional is absent. Otherwise, we do need to match.
                    if has_required_anywhere {
                        true
                    } else {
                        // When there are no required terms, we still need to enforce that all keywords
                        // within a single Term are present (AND logic within a Term).
                        // This ensures that for a term like "JWTMiddleware" which gets tokenized to
                        // ["jwt", "middleware"], both parts must be present.

                        // Check if any keywords are present
                        let any_present = keywords.iter().any(|kw| {
                            term_indices
                                .get(kw)
                                .map(|idx| matched_terms.contains(idx))
                                .unwrap_or(false)
                        });

                        // If no keywords are present, the term doesn't match
                        if !any_present {
                            return false;
                        }

                        // If at least one keyword is present, require all keywords to be present
                        // This maintains the AND relationship between keywords in a single Term
                        all_present
                    }
                }
            }
            Expr::And(left, right) => {
                let lval = left.evaluate_with_has_required(
                    matched_terms,
                    term_indices,
                    ignore_negatives,
                    has_required_anywhere,
                );
                let rval = right.evaluate_with_has_required(
                    matched_terms,
                    term_indices,
                    ignore_negatives,
                    has_required_anywhere,
                );
                if debug_mode {
                    println!(
                        "DEBUG: AND => left={}, right={}, result={}",
                        lval,
                        rval,
                        lval && rval
                    );
                }
                lval && rval
            }
            Expr::Or(left, right) => {
                let lval = left.evaluate_with_has_required(
                    matched_terms,
                    term_indices,
                    ignore_negatives,
                    has_required_anywhere,
                );
                let rval = right.evaluate_with_has_required(
                    matched_terms,
                    term_indices,
                    ignore_negatives,
                    has_required_anywhere,
                );

                if debug_mode {
                    println!(
                        "DEBUG: OR => left={}, right={}, result={}",
                        lval,
                        rval,
                        lval || rval
                    );
                }
                lval || rval
            }
        }
    }

    // PHASE 3C OPTIMIZATION: Fast-path evaluation for simple queries
    pub fn evaluate_fast_path(
        &self,
        matched_terms: &HashSet<usize>,
        plan: &crate::search::query::QueryPlan,
    ) -> Option<bool> {
        // Fast path for simple single-term queries
        if plan.is_simple_query {
            return Some(!matched_terms.is_empty());
        }

        // Fast path: if no terms matched and query requires some terms
        if matched_terms.is_empty() && !plan.has_only_excluded_terms {
            return Some(false);
        }

        // Fast path: if all required terms are missing
        if !plan.required_terms_indices.is_empty() {
            let has_all_required = plan
                .required_terms_indices
                .iter()
                .all(|idx| matched_terms.contains(idx));
            if !has_all_required {
                return Some(false);
            }
        }

        None // Fall back to full evaluation
    }

    // PHASE 3C OPTIMIZATION: Cached evaluation
    pub fn evaluate_with_cache(
        &self,
        matched_terms: &HashSet<usize>,
        plan: &crate::search::query::QueryPlan,
    ) -> bool {
        // Try fast path first
        if let Some(result) = self.evaluate_fast_path(matched_terms, plan) {
            return result;
        }

        // Compute cache key from matched terms
        let cache_key = compute_evaluation_key(matched_terms);

        // Check cache
        if let Ok(mut cache) = plan.evaluation_cache.lock() {
            if let Some(&cached_result) = cache.peek(&cache_key) {
                return cached_result;
            }

            // Perform full evaluation
            let result = self.evaluate(matched_terms, &plan.term_indices, false);

            // Cache the result
            cache.put(cache_key, result);

            result
        } else {
            // If we can't lock the cache, just evaluate without caching
            self.evaluate(matched_terms, &plan.term_indices, false)
        }
    }

    /// Evaluate whether a set of matched term indices satisfies this logical expression.
    ///
    /// - Term: check if **all** of its keywords are present (optional/required), or
    ///   if **none** are present (excluded).
    /// - AND => both sides must match.
    /// - OR => at least one side must match.
    /// - `ignore_negatives` => if true, excluded terms are basically ignored (they don't exclude).
    /// - Field is **ignored** in evaluation, per request.
    pub fn evaluate(
        &self,
        matched_terms: &HashSet<usize>,
        term_indices: &HashMap<String, usize>,
        ignore_negatives: bool,
    ) -> bool {
        // Early termination optimization
        if matched_terms.is_empty() && !self.is_only_excluded_terms() {
            return false;
        }

        let debug_mode = std::env::var("PROBE_DEBUG").unwrap_or_default() == "1";

        // If ignoring negatives, let's ensure that all required terms are present up front.
        // (We skip enforcing them again for each subtree.)
        if ignore_negatives {
            fn collect_required(expr: &Expr) -> Vec<String> {
                match expr {
                    Expr::Term {
                        keywords,
                        required,
                        excluded,
                        ..
                    } => {
                        if *required && !*excluded {
                            keywords.clone()
                        } else {
                            vec![]
                        }
                    }
                    Expr::And(left, right) => {
                        let mut out = collect_required(left);
                        out.extend(collect_required(right));
                        out
                    }
                    Expr::Or(left, right) => {
                        let mut out = collect_required(left);
                        out.extend(collect_required(right));
                        out
                    }
                }
            }
            let required_terms = collect_required(self);
            if debug_mode && !required_terms.is_empty() {
                println!("DEBUG: Required terms (ignoring negatives): {required_terms:?}");
            }
            for term in &required_terms {
                if let Some(&idx) = term_indices.get(term) {
                    if !matched_terms.contains(&idx) {
                        if debug_mode {
                            println!("DEBUG: Missing required term '{term}' (idx={idx})");
                        }
                        return false;
                    }
                } else {
                    // If we can't find that required term at all, fail immediately
                    return false;
                }
            }
        }

        // Compute once for the entire query whether any term is required
        let has_required_anywhere = self.has_required_term();

        if debug_mode {
            println!("DEBUG: Evaluating => {self:?}");
            println!("DEBUG: matched_terms => {matched_terms:?}");
            println!("DEBUG: term_indices => {term_indices:?}");
            println!("DEBUG: Expression has_required_anywhere? {has_required_anywhere}");
        }

        // Delegate final checks to our helper, which references has_required_anywhere
        self.evaluate_with_has_required(
            matched_terms,
            term_indices,
            ignore_negatives,
            has_required_anywhere,
        )
    }
}

impl std::fmt::Display for Expr {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Expr::Term {
                keywords,
                field,
                required,
                excluded,
                exact,
            } => {
                let prefix = if *required {
                    "+"
                } else if *excluded {
                    "-"
                } else {
                    ""
                };
                let field_prefix = if let Some(ref field_name) = field {
                    format!("{field_name}:")
                } else {
                    String::new()
                };
                // If there's exactly one keyword and it's exact => show it quoted
                // If multiple or not exact => "quoted" with joined keywords
                if keywords.len() == 1 && *exact {
                    write!(f, "{}{}\"{}\"", prefix, field_prefix, keywords[0])
                } else if keywords.len() == 1 {
                    write!(f, "{}{}{}", prefix, field_prefix, keywords[0])
                } else {
                    write!(f, "{}{}\"{}\"", prefix, field_prefix, keywords.join(" "))
                }
            }
            Expr::And(left, right) => write!(f, "({left} AND {right})"),
            Expr::Or(left, right) => write!(f, "({left} OR {right})"),
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum Token {
    Plus,                 // '+'
    Minus,                // '-'
    And,                  // 'AND'
    Or,                   // 'OR'
    LParen,               // '('
    RParen,               // ')'
    Colon,                // ':'
    Ident(String),        // alphanumeric / underscore / dot
    QuotedString(String), // raw string inside quotes
}

/// A simple error type for parsing/tokenizing.
#[derive(Debug)]
pub enum ParseError {
    #[allow(dead_code)]
    UnexpectedChar(char),
    UnexpectedEndOfInput,
    UnexpectedToken(Token),
    Generic(String),
}
impl std::fmt::Display for ParseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ParseError::UnexpectedChar(c) => write!(f, "Unexpected character '{c}'"),
            ParseError::UnexpectedEndOfInput => write!(f, "Unexpected end of input"),
            ParseError::UnexpectedToken(t) => write!(f, "Unexpected token '{t:?}'"),
            ParseError::Generic(s) => write!(f, "{s}"),
        }
    }
}

impl std::error::Error for ParseError {}

/// Tokenize input string into a vector of tokens
fn tokenize(input: &str) -> Result<Vec<Token>, ParseError> {
    let mut chars = input.chars().peekable();
    let mut tokens = Vec::new();
    let debug_mode = std::env::var("PROBE_DEBUG").unwrap_or_default() == "1";

    while let Some(&ch) = chars.peek() {
        match ch {
            c if c.is_whitespace() => {
                chars.next();
            }
            '+' => {
                tokens.push(Token::Plus);
                chars.next();
            }
            '-' => {
                tokens.push(Token::Minus);
                chars.next();
            }
            '(' => {
                tokens.push(Token::LParen);
                chars.next();
            }
            ')' => {
                tokens.push(Token::RParen);
                chars.next();
            }
            ':' => {
                tokens.push(Token::Colon);
                chars.next();
            }
            '"' => {
                chars.next(); // consume the opening quote
                let quoted_string = lex_quoted_string(&mut chars)?;
                tokens.push(Token::QuotedString(quoted_string));
            }
            _ => {
                // If it starts with alphanumeric, underscore, or dot => parse identifier
                if ch.is_alphanumeric() || ch == '_' || ch == '.' {
                    let ident = lex_identifier(&mut chars);
                    let ident_upper = ident.to_ascii_uppercase();
                    if ident_upper == "AND" {
                        tokens.push(Token::And);
                    } else if ident_upper == "OR" {
                        tokens.push(Token::Or);
                    } else {
                        tokens.push(Token::Ident(ident));
                    }
                } else {
                    // Skip unknown characters
                    if debug_mode {
                        println!("DEBUG: Skipping unknown character '{ch}'");
                    }
                    chars.next();
                }
            }
        }
    }

    if tokens.is_empty() {
        return Err(ParseError::Generic(
            "No valid tokens found in input".to_string(),
        ));
    }
    Ok(tokens)
}

/// Lex a quoted string, allowing `\"` to escape quotes
fn lex_quoted_string(chars: &mut Peekable<Chars>) -> Result<String, ParseError> {
    let mut buf = String::new();
    let mut escaped = false;

    while let Some(&ch) = chars.peek() {
        if escaped {
            buf.push(ch);
            escaped = false;
            chars.next();
        } else if ch == '\\' {
            escaped = true;
            chars.next();
        } else if ch == '"' {
            chars.next(); // consume the closing quote
            return Ok(buf);
        } else {
            buf.push(ch);
            chars.next();
        }
    }
    // If we get here, we ran out of characters before finding a closing quote
    Err(ParseError::UnexpectedEndOfInput)
}

fn lex_identifier(chars: &mut Peekable<Chars>) -> String {
    let mut buf = String::new();
    while let Some(&ch) = chars.peek() {
        if ch.is_alphanumeric() || ch == '_' || ch == '.' || ch == '-' {
            buf.push(ch);
            chars.next();
        } else {
            break;
        }
    }
    buf
}

// Adjust paths to match your project structure
use probe_code::search::tokenization::{add_special_term, tokenize as custom_tokenize};

struct Parser {
    tokens: Vec<Token>,
    pos: usize,
}

impl Parser {
    fn new(tokens: Vec<Token>) -> Self {
        Parser { tokens, pos: 0 }
    }

    fn peek(&self) -> Option<&Token> {
        self.tokens.get(self.pos)
    }

    fn next(&mut self) -> Option<Token> {
        let t = self.peek()?.clone();
        self.pos += 1;
        Some(t)
    }

    fn expect(&mut self, expected: &Token) -> Result<Token, ParseError> {
        match self.peek() {
            Some(t) if t == expected => Ok(self.next().unwrap()),
            Some(t) => Err(ParseError::UnexpectedToken(t.clone())),
            None => Err(ParseError::UnexpectedEndOfInput),
        }
    }

    fn parse_expr(&mut self) -> Result<Expr, ParseError> {
        let expr = self.parse_or_expr()?;
        // If leftover tokens remain, we ignore them for now
        Ok(expr)
    }

    fn parse_or_expr(&mut self) -> Result<Expr, ParseError> {
        let debug_mode = std::env::var("PROBE_DEBUG").unwrap_or_default() == "1";
        if debug_mode {
            println!("DEBUG: parse_or_expr => pos={pos}", pos = self.pos);
        }

        let mut left = self.parse_and_expr()?;

        while let Some(Token::Or) = self.peek() {
            self.next(); // consume 'OR'
            let right = self.parse_and_expr()?;
            left = Expr::Or(Box::new(left), Box::new(right));
            if debug_mode {
                println!("DEBUG: OR => {left:?}");
            }
        }
        Ok(left)
    }

    fn parse_and_expr(&mut self) -> Result<Expr, ParseError> {
        let debug_mode = std::env::var("PROBE_DEBUG").unwrap_or_default() == "1";
        if debug_mode {
            println!("DEBUG: parse_and_expr => pos={pos}", pos = self.pos);
        }

        let mut left = self.parse_factor()?;

        while let Some(token) = self.peek() {
            match token {
                // Explicit "AND"
                Token::And => {
                    self.next(); // consume 'AND'
                    let right = self.parse_factor()?;
                    left = Expr::And(Box::new(left), Box::new(right));
                    if debug_mode {
                        println!("DEBUG: AND => {left:?}");
                    }
                }
                // If we see "OR", break so parse_or_expr can handle it
                Token::Or => {
                    break;
                }
                // If next token is a plus or minus, interpret as an AND
                Token::Plus | Token::Minus => {
                    let right = self.parse_factor()?;
                    left = Expr::And(Box::new(left), Box::new(right));
                    if debug_mode {
                        println!("DEBUG: forced AND => {left:?}");
                    }
                }
                // Otherwise (Ident, QuotedString, LParen) => implicit combos
                Token::Ident(_) | Token::QuotedString(_) | Token::LParen => {
                    let right = self.parse_factor()?;
                    // True Lucene/Elasticsearch semantics: implicit combinations are always OR
                    // The + and - operators only affect individual terms, not the combination logic
                    left = Expr::Or(Box::new(left), Box::new(right));
                    if debug_mode {
                        println!("DEBUG: implicit OR => {left:?}");
                    }
                }
                _ => break,
            }
        }

        Ok(left)
    }

    fn parse_factor(&mut self) -> Result<Expr, ParseError> {
        match self.peek() {
            Some(Token::LParen) => {
                self.next(); // consume '('
                let expr = self.parse_expr()?;
                self.expect(&Token::RParen)?;
                Ok(expr)
            }
            _ => self.parse_prefixed_term(),
        }
    }

    fn parse_prefixed_term(&mut self) -> Result<Expr, ParseError> {
        let mut required = false;
        let mut excluded = false;
        let debug_mode = std::env::var("PROBE_DEBUG").unwrap_or_default() == "1";

        match self.peek() {
            Some(Token::Plus) => {
                required = true;
                self.next();
            }
            Some(Token::Minus) => {
                excluded = true;
                self.next();
            }
            _ => {}
        }

        let primary_expr = self.parse_primary()?;
        // If it's a Term => update its required/excluded
        if let Expr::Term {
            keywords,
            field,
            required: _,
            excluded: _,
            exact,
        } = primary_expr
        {
            // If exact or excluded => skip further tokenization
            let final_keywords = if exact || excluded {
                // Mark them special (no splitting)
                for kw in &keywords {
                    add_special_term(kw);
                }
                keywords
            } else {
                // Apply your custom tokenization
                let mut expanded = Vec::new();
                for kw in &keywords {
                    let splitted = custom_tokenize(kw);
                    // Only add non-empty terms
                    expanded.extend(splitted.into_iter().filter(|s| !s.is_empty()));
                }
                // If all terms were filtered out (e.g., all were stop words),
                // return an empty vector which will be handled properly
                expanded
            };

            if debug_mode {
                println!("DEBUG: parse_prefixed_term => required={required}, excluded={excluded}, final_keywords={final_keywords:?}");
            }

            Ok(Expr::Term {
                keywords: final_keywords,
                field,
                required,
                excluded,
                exact,
            })
        } else {
            // If it's a sub-expression in parentheses or something else, just return it
            Ok(primary_expr)
        }
    }

    fn parse_primary(&mut self) -> Result<Expr, ParseError> {
        let debug_mode = std::env::var("PROBE_DEBUG").unwrap_or_default() == "1";

        match self.peek() {
            // Quoted => exact
            Some(Token::QuotedString(s)) => {
                let val = s.clone();
                self.next();
                if debug_mode {
                    println!("DEBUG: QuotedString => {val}");
                }
                Ok(Expr::Term {
                    keywords: vec![val],
                    field: None,
                    required: false,
                    excluded: false,
                    exact: true,
                })
            }
            // Possibly field:term
            Some(Token::Ident(_)) => {
                let Token::Ident(first) = self.next().unwrap() else {
                    unreachable!()
                };
                if debug_mode {
                    println!("DEBUG: Ident => {first}");
                }
                if let Some(Token::Colon) = self.peek() {
                    // We have "field:"
                    self.next(); // consume colon
                                 // Next could be ident or quoted
                    match self.peek() {
                        Some(Token::Ident(ident2)) => {
                            let val2 = ident2.clone();
                            self.next();
                            Ok(Expr::Term {
                                keywords: vec![val2],
                                field: Some(first),
                                required: false,
                                excluded: false,
                                exact: false,
                            })
                        }
                        Some(Token::QuotedString(qs)) => {
                            let qval = qs.clone();
                            self.next();
                            Ok(Expr::Term {
                                keywords: vec![qval],
                                field: Some(first),
                                required: false,
                                excluded: false,
                                exact: true,
                            })
                        }
                        // If nothing or other token => empty term
                        _ => Ok(Expr::Term {
                            keywords: vec![],
                            field: Some(first),
                            required: false,
                            excluded: false,
                            exact: false,
                        }),
                    }
                } else {
                    // Just a plain ident
                    Ok(Expr::Term {
                        keywords: vec![first],
                        field: None,
                        required: false,
                        excluded: false,
                        exact: false,
                    })
                }
            }
            Some(t) => Err(ParseError::UnexpectedToken(t.clone())),
            None => Err(ParseError::UnexpectedEndOfInput),
        }
    }
}

/// Parse the query string into an AST
pub fn parse_query(input: &str, exact: bool) -> Result<Expr, ParseError> {
    let debug_mode = std::env::var("PROBE_DEBUG").unwrap_or_default() == "1";

    if debug_mode {
        println!("DEBUG: parse_query('{input}', exact={exact})");
    }

    // If exact search is enabled, treat the entire query as a single term
    if exact {
        if debug_mode {
            println!("DEBUG: Exact search enabled, treating query as a single term");
        }
        return Ok(Expr::Term {
            keywords: vec![input.to_string()],
            field: None,
            required: false,
            excluded: false,
            exact: true,
        });
    }

    // Tokenize
    let tokens_result = tokenize(input);
    if debug_mode {
        println!("DEBUG: Tokens => {tokens_result:?}");
    }

    // If tokenization fails => fallback
    let tokens = match tokens_result {
        Ok(ts) => ts,
        Err(_) => {
            let cleaned_input = input
                .chars()
                .filter(|&c| c.is_alphanumeric() || c.is_whitespace() || c == '_' || c == '.')
                .collect::<String>();
            if cleaned_input.trim().is_empty() {
                return Err(ParseError::Generic("No valid tokens found".to_string()));
            }
            let keywords = cleaned_input
                .split_whitespace()
                .map(|s| s.to_lowercase())
                .collect::<Vec<String>>();
            return Ok(Expr::Term {
                keywords,
                field: None,
                required: false,
                excluded: false,
                exact: false,
            });
        }
    };

    // Parse into AST
    let mut parser = Parser::new(tokens);
    let parsed = parser.parse_expr();

    if parsed.is_err() {
        // If parse fails => fallback to any Ident tokens
        let idents = parser
            .tokens
            .iter()
            .filter_map(|t| match t {
                Token::Ident(s) => Some(s.clone()),
                _ => None,
            })
            .collect::<Vec<_>>();
        if idents.is_empty() {
            return Err(ParseError::Generic(
                "No valid identifiers found".to_string(),
            ));
        }
        return Ok(Expr::Term {
            keywords: idents,
            field: None,
            required: false,
            excluded: false,
            exact: false,
        });
    }

    // Otherwise success
    Ok(parsed.unwrap())
}

/// Backward compatibility wrapper for parse_query
#[allow(dead_code)]
pub fn parse_query_compat(input: &str) -> Result<Expr, ParseError> {
    parse_query(input, false)
}

// Make parse_query_test available for tests in other modules
#[allow(dead_code)]
pub fn parse_query_test(input: &str) -> Result<Expr, ParseError> {
    parse_query(input, false)
}

#[cfg(test)]
mod tests {
    include!("elastic_query_tests.rs");
    include!("elastic_query_evaluate_tests.rs");
    include!("elastic_query_tokenization_tests.rs");
}
