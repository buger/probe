#[cfg(test)]
mod token_limiter_failure_tests {
    use super::super::search_limiter::apply_limits;
    use crate::models::SearchResult;
    use crate::search::search_tokens::count_block_tokens;

    /// Helper function to create a SearchResult with specific code content
    fn create_test_result(code: &str, rank: Option<usize>) -> SearchResult {
        SearchResult {
            file: "test.rs".to_string(),
            lines: (1, 1),
            node_type: "test".to_string(),
            code: code.to_string(),
            matched_by_filename: None,
            rank,
            score: None,
            tfidf_score: Some(1.0),
            bm25_score: Some(1.0),
            tfidf_rank: None,
            bm25_rank: None,
            new_score: None,
            hybrid2_rank: None,
            combined_score_rank: None,
            file_unique_terms: None,
            file_total_matches: None,
            file_match_rank: None,
            block_unique_terms: None,
            block_total_matches: None,
            parent_file_id: None,
            block_id: None,
            matched_keywords: None,
            tokenized_content: None,
            lsp_info: None,
        }
    }

    #[test]
    #[should_panic(expected = "Token limit severely exceeded")]
    fn test_compressed_code_causes_severe_overrun() {
        // This test demonstrates that compressed code can cause massive token overruns
        let compressed_blocks = [
            "let[a,b,c,d,e,f,g,h,i,j,k,l,m,n,o,p,q,r,s,t,u,v,w,x,y,z]=[1,2,3,4,5,6,7,8,9,10,11,12,13,14,15,16,17,18,19,20,21,22,23,24,25,26];",
            "const{x,y,z,w}={x:1,y:2,z:3,w:4};if(x>y){z=x+y}else{z=x-y}for(let i=0;i<10;i++){console.log(i)}",
            "()=>({a:1,b:2,c:3,d:4,e:5}).map((k,v)=>k+v).filter(x=>x>5).reduce((a,b)=>a+b,0)",
        ];

        let results: Vec<SearchResult> = compressed_blocks
            .iter()
            .enumerate()
            .map(|(i, code)| create_test_result(code, Some(i)))
            .collect();

        let token_limit = 100;
        let limited = apply_limits(results, None, None, Some(token_limit));

        // Calculate actual token count to verify overrun
        let actual_total_tokens: usize = limited
            .results
            .iter()
            .map(|r| count_block_tokens(&r.code))
            .sum();

        if actual_total_tokens > token_limit {
            let overrun_percent =
                ((actual_total_tokens - token_limit) as f64 / token_limit as f64) * 100.0;
            panic!(
                "Token limit severely exceeded: {actual_total_tokens} actual tokens vs {token_limit} limit ({overrun_percent:.1}% overrun)"
            );
        }
    }

    #[test]
    #[should_panic(expected = "90% threshold bypassed")]
    fn test_90_percent_threshold_bypass() {
        // Create blocks that stay under 90% estimation but exceed 100% actual
        // The key is to use content that tokenizes inefficiently (many tokens per character)
        let deceptive_blocks = [
            "function normal() { return 42; }",  // Normal code (~8 tokens, 33 chars)
            "const value = getData();",          // Normal code (~6 tokens, 25 chars)  
            // Killer block: lots of individual tokens that don't compress well
            "const a1=1,a2=2,a3=3,a4=4,a5=5,a6=6,a7=7,a8=8,a9=9,a10=10,a11=11,a12=12,a13=13,a14=14,a15=15,a16=16,a17=17,a18=18,a19=19,a20=20,a21=21,a22=22,a23=23,a24=24,a25=25,a26=26,a27=27,a28=28,a29=29,a30=30,a31=31,a32=32,a33=33,a34=34,a35=35,a36=36,a37=37,a38=38,a39=39,a40=40,a41=41,a42=42,a43=43,a44=44,a45=45,a46=46,a47=47,a48=48,a49=49,a50=50;",
        ];

        let results: Vec<SearchResult> = deceptive_blocks
            .iter()
            .enumerate()
            .map(|(i, code)| create_test_result(code, Some(i)))
            .collect();

        let token_limit = 150;
        let ninety_percent_threshold = (token_limit as f64 * 0.9) as usize;

        // Calculate what the estimation would show
        let estimated_total: usize = results.iter().map(|r| (r.code.len() / 4).max(1)).sum();

        let actual_total: usize = results.iter().map(|r| count_block_tokens(&r.code)).sum();

        if estimated_total < ninety_percent_threshold && actual_total > token_limit {
            panic!(
                "90% threshold bypassed: {estimated_total} estimated < {ninety_percent_threshold} threshold, but {actual_total} actual > {token_limit} limit"
            );
        }
    }

    #[test]
    fn test_symbol_heavy_code_underestimation() {
        let symbol_blocks = [
            "()[]{}()[]{}()",
            "{}[](){}[](){}[](){}[](){}[](){}[](){}[](){}[](){}",
            "(((())))[[[[]]]]{{{{}}}}",
        ];

        let results: Vec<SearchResult> = symbol_blocks
            .iter()
            .enumerate()
            .map(|(i, code)| create_test_result(code, Some(i)))
            .collect();

        // Calculate estimation vs actual
        let estimated_total: usize = results.iter().map(|r| (r.code.len() / 4).max(1)).sum();

        let actual_total: usize = results.iter().map(|r| count_block_tokens(&r.code)).sum();

        let error_percent =
            ((estimated_total as f64 - actual_total as f64) / actual_total as f64) * 100.0;

        // Symbol-heavy code should show severe underestimation (>50% error)
        assert!(
            error_percent < -50.0,
            "Expected severe underestimation for symbol-heavy code, got {error_percent:.1}% error"
        );
    }

    #[test]
    fn test_accumulation_of_estimation_errors() {
        // Create many small blocks with low bytes/token ratios
        let mut small_blocks = Vec::new();
        for i in 0..25 {
            small_blocks.push(format!("a{i}=1;b{i}=2;c{i}=3;"));
        }

        let results: Vec<SearchResult> = small_blocks
            .iter()
            .enumerate()
            .map(|(i, code)| create_test_result(code, Some(i)))
            .collect();

        let token_limit = 150;
        let limited = apply_limits(results.clone(), None, None, Some(token_limit));

        let reported_tokens = limited
            .limits_applied
            .as_ref()
            .map(|l| l.total_tokens)
            .unwrap_or(0);

        let actual_tokens: usize = limited
            .results
            .iter()
            .map(|r| count_block_tokens(&r.code))
            .sum();

        // Check if accumulation of small errors leads to significant undercount
        if actual_tokens > reported_tokens {
            let undercount_percent =
                ((actual_tokens - reported_tokens) as f64 / reported_tokens as f64) * 100.0;
            assert!(
                undercount_percent < 20.0,
                "Accumulation of errors caused {undercount_percent}% undercount ({actual_tokens} actual vs {reported_tokens} reported)"
            );
        }
    }

    #[test]
    fn test_unicode_tokenization_surprises() {
        let unicode_blocks = [
            "变量一=1;变量二=2;变量三=3;",
            "const emoji = '🚀🎉💻';",
            "函数名() { 返回值 = 42; }",
        ];

        let results: Vec<SearchResult> = unicode_blocks
            .iter()
            .enumerate()
            .map(|(i, code)| create_test_result(code, Some(i)))
            .collect();

        // Calculate bytes/token ratio for Unicode content
        let total_bytes: usize = results.iter().map(|r| r.code.len()).sum();
        let total_tokens: usize = results.iter().map(|r| count_block_tokens(&r.code)).sum();
        let actual_ratio = total_bytes as f64 / total_tokens as f64;

        // Unicode content often has lower bytes/token ratios than expected
        println!("Unicode bytes/token ratio: {actual_ratio:.2}");

        // This test documents the behavior rather than asserting failure
        // but shows that Unicode can cause unexpected tokenization
        assert!(actual_ratio > 0.0, "Sanity check: ratio should be positive");
    }

    #[test]
    fn test_mixed_content_unpredictability() {
        // Mix normal and problematic content to show unpredictable behavior
        let mixed_blocks = [
            "// This is a normal comment that should have decent bytes/token ratio",
            "function calculateValue(input) { return input * 2; }",
            "let[a,b,c]=[1,2,3];", // Compressed
            "const MESSAGE = 'This is a longer string with more typical natural language content';",
            "()[]{}()[]{}()", // Symbol soup
        ];

        let results: Vec<SearchResult> = mixed_blocks
            .iter()
            .enumerate()
            .map(|(i, code)| create_test_result(code, Some(i)))
            .collect();

        // Show how mixed content makes the overall ratio unpredictable
        let total_bytes: usize = results.iter().map(|r| r.code.len()).sum();
        let total_tokens: usize = results.iter().map(|r| count_block_tokens(&r.code)).sum();
        let actual_ratio = total_bytes as f64 / total_tokens as f64;

        println!("Mixed content bytes/token ratio: {actual_ratio:.2}");

        // The 4.0 assumption could be wrong in either direction
        let error_from_assumption = ((4.0 - actual_ratio) / actual_ratio * 100.0).abs();

        // Document that mixed content makes estimation unreliable
        if error_from_assumption > 15.0 {
            println!("WARNING: Mixed content shows {error_from_assumption:.1}% deviation from 4.0 assumption");
        }

        assert!(actual_ratio > 0.0, "Sanity check: ratio should be positive");
    }

    #[test]
    fn test_exact_90_percent_threshold_behavior() {
        // Test the exact behavior at the 90% threshold
        let token_limit = 100;
        let threshold_90_percent = (token_limit as f64 * 0.9) as usize; // 90 tokens

        // Create content that gets us exactly to 90% with estimation
        // but exceeds 100% with actual tokens
        let crafted_content = "a=1;b=2;".repeat(15); // Should estimate ~22-23 tokens but be much more

        let _results = vec![create_test_result(&crafted_content, Some(0))];

        let estimated_tokens = (crafted_content.len() / 4).max(1);
        let actual_tokens = count_block_tokens(&crafted_content);

        println!(
            "Crafted content: {} bytes, {} estimated, {} actual",
            crafted_content.len(),
            estimated_tokens,
            actual_tokens
        );

        // Verify our setup creates the problematic scenario
        if estimated_tokens < threshold_90_percent && actual_tokens > token_limit {
            println!("SUCCESS: Created scenario where estimation ({estimated_tokens}) < 90% threshold ({threshold_90_percent}) but actual ({actual_tokens}) > limit ({token_limit})");
        }
    }
}
