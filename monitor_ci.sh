#!/bin/bash

# Monitor CI and fix issues until all checks pass

PR_NUMBER=103
MAX_ATTEMPTS=20
ATTEMPT=0

while [ $ATTEMPT -lt $MAX_ATTEMPTS ]; do
    ATTEMPT=$((ATTEMPT + 1))
    echo "=== CI Check Attempt $ATTEMPT of $MAX_ATTEMPTS ==="
    
    # Wait 5 minutes before checking (skip on first attempt)
    if [ $ATTEMPT -gt 1 ]; then
        echo "Waiting 5 minutes before checking CI status..."
        sleep 300
    fi
    
    # Check CI status
    echo "Checking CI status..."
    FAILED_COUNT=$(gh pr checks $PR_NUMBER 2>/dev/null | grep -c "fail" || echo "0")
    PENDING_COUNT=$(gh pr checks $PR_NUMBER 2>/dev/null | grep -c "pending" || echo "0")
    
    echo "Failed checks: $FAILED_COUNT"
    echo "Pending checks: $PENDING_COUNT"
    
    # If all checks pass, we're done
    if [ "$FAILED_COUNT" -eq 0 ] && [ "$PENDING_COUNT" -eq 0 ]; then
        echo "✅ All CI checks are passing!"
        exit 0
    fi
    
    # If checks are still pending, continue waiting
    if [ "$PENDING_COUNT" -gt 0 ]; then
        echo "Checks still pending, will check again in 5 minutes..."
        continue
    fi
    
    # If there are failures, analyze and fix
    if [ "$FAILED_COUNT" -gt 0 ]; then
        echo "Analyzing failures..."
        
        # Check for formatting issues
        if gh pr checks $PR_NUMBER 2>/dev/null | grep -q "Check formatting.*fail"; then
            echo "Fixing formatting issues..."
            cargo fmt --all
            
            if git diff --quiet; then
                echo "No formatting changes needed"
            else
                git add -A
                git commit -m "Fix code formatting (automated)

🤖 Generated with [Claude Code](https://claude.ai/code)

Co-Authored-By: Claude <noreply@anthropic.com>"
                git push origin restructure-lsp-daemon-root
                echo "Pushed formatting fixes"
                continue
            fi
        fi
        
        # Check for clippy issues
        if gh pr checks $PR_NUMBER 2>/dev/null | grep -q "Lint with clippy.*fail"; then
            echo "Fixing clippy issues..."
            cargo clippy --fix --allow-dirty --all-targets --all-features 2>/dev/null
            
            if git diff --quiet; then
                echo "No clippy fixes needed"
            else
                git add -A
                git commit -m "Fix clippy warnings (automated)

🤖 Generated with [Claude Code](https://claude.ai/code)

Co-Authored-By: Claude <noreply@anthropic.com)"
                git push origin restructure-lsp-daemon-root
                echo "Pushed clippy fixes"
                continue
            fi
        fi
        
        # Check for test failures
        if gh pr checks $PR_NUMBER 2>/dev/null | grep -q "Run tests.*fail"; then
            echo "Tests are failing - manual intervention may be needed"
            gh run list --branch restructure-lsp-daemon-root --limit 1
            # For now, we can't automatically fix test failures
            echo "Please check the test logs manually"
        fi
    fi
done

echo "❌ Maximum attempts reached. CI issues persist."
exit 1