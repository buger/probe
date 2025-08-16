use anyhow::Result;
use lsp_daemon::{
    get_default_socket_path, start_daemon_background, DaemonRequest, DaemonResponse, DaemonStatus,
    IpcStream, MessageCodec,
};
use std::fs;
use std::path::{Path, PathBuf};
use tempfile::TempDir;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::time::{sleep, Duration};
use uuid::Uuid;

/// Integration test for multi-workspace LSP daemon functionality
#[tokio::test]
#[ignore = "Requires gopls with proper Go environment setup - run with --ignored to test"]
async fn test_multi_workspace_go_projects() -> Result<()> {
    // Clean up any existing daemon
    let _ = std::process::Command::new("pkill")
        .args(["-f", "lsp-daemon"])
        .output();

    sleep(Duration::from_millis(500)).await;

    // Create temporary workspaces
    let temp_dir = TempDir::new()?;
    let workspace1 = setup_go_project(&temp_dir, "project1", GO_PROJECT1_CODE).await?;
    let workspace2 = setup_go_project(&temp_dir, "project2", GO_PROJECT2_CODE).await?;
    let workspace3 = setup_go_project(&temp_dir, "project3", GO_PROJECT3_CODE).await?;

    // Start daemon
    start_daemon_background().await?;
    sleep(Duration::from_millis(2000)).await; // Give more time for daemon to fully start

    let socket_path = get_default_socket_path();

    // Test workspace 1: Database project
    test_project_analysis(&socket_path, &workspace1, &[("main", 25)]).await?;
    test_project_analysis(&socket_path, &workspace1, &[("Connect", 14)]).await?;

    // Test workspace 2: Web server project
    test_project_analysis(&socket_path, &workspace2, &[("main", 25)]).await?;
    test_project_analysis(&socket_path, &workspace2, &[("Start", 16)]).await?;

    // Test workspace 3: Calculator project
    test_project_analysis(&socket_path, &workspace3, &[("main", 29)]).await?;
    test_project_analysis(&socket_path, &workspace3, &[("Add", 14)]).await?;

    // Verify daemon status shows multiple workspaces
    let status = get_daemon_status(&socket_path).await?;

    // Should have at least 3 Go pools (one per workspace)
    let go_pools = status
        .pools
        .iter()
        .filter(|p| p.language.as_str() == "Go")
        .count();
    assert!(
        go_pools >= 3,
        "Expected at least 3 Go pools, got {go_pools}"
    );

    println!("✅ Multi-workspace test completed successfully!");
    println!("   - {} workspaces tested", 3);
    println!("   - {go_pools} Go language pools active");
    println!("   - Total requests processed: {}", status.total_requests);

    Ok(())
}

async fn setup_go_project(temp_dir: &TempDir, name: &str, code: &str) -> Result<PathBuf> {
    let project_dir = temp_dir.path().join(name);
    fs::create_dir_all(&project_dir)?;

    // Create go.mod
    fs::write(
        project_dir.join("go.mod"),
        format!("module {name}\n\ngo 1.21\n"),
    )?;

    // Create main.go
    fs::write(project_dir.join("main.go"), code)?;

    // Initialize the Go module properly by running go mod tidy
    // This ensures gopls can find package metadata
    let output = std::process::Command::new("go")
        .args(["mod", "tidy"])
        .current_dir(&project_dir)
        .output();

    if let Err(e) = output {
        println!("Warning: Failed to run 'go mod tidy' in {project_dir:?}: {e}");
    }

    Ok(project_dir)
}

async fn test_project_analysis(
    socket_path: &str,
    workspace: &Path,
    expected_callers: &[(&str, u32)],
) -> Result<()> {
    // Retry connection up to 5 times with exponential backoff
    let mut stream = None;
    for attempt in 0..5 {
        match IpcStream::connect(socket_path).await {
            Ok(s) => {
                stream = Some(s);
                break;
            }
            Err(e) if attempt < 4 => {
                println!(
                    "Connection attempt {} failed: {}, retrying...",
                    attempt + 1,
                    e
                );
                sleep(Duration::from_millis(1000 * (attempt + 1) as u64)).await;
            }
            Err(e) => return Err(e),
        }
    }

    let mut stream = stream.unwrap();

    let request = DaemonRequest::CallHierarchy {
        request_id: Uuid::new_v4(),
        file_path: workspace.join("main.go"),
        line: 5,   // Line number where the function might be
        column: 0, // Column number
        workspace_hint: Some(workspace.to_path_buf()),
    };

    let encoded = MessageCodec::encode(&request)?;
    stream.write_all(&encoded).await?;

    // Read response with timeout
    let mut response_data = vec![0u8; 8192];
    let n =
        tokio::time::timeout(Duration::from_secs(60), stream.read(&mut response_data)).await??;
    response_data.truncate(n);

    match MessageCodec::decode_response(&response_data)? {
        DaemonResponse::CallHierarchy { result, .. } => {
            println!(
                "✅ Call hierarchy in {:?}: {} incoming calls",
                workspace.file_name().unwrap(),
                result.incoming.len()
            );

            // Verify expected callers
            assert_eq!(
                result.incoming.len(),
                expected_callers.len(),
                "Expected {} callers, got {}",
                expected_callers.len(),
                result.incoming.len()
            );

            for (expected_caller, expected_line) in expected_callers {
                let found = result.incoming.iter().any(|call| {
                    call.from.name.contains(expected_caller)
                        && call.from_ranges.iter().any(|range| {
                            range.start.line >= expected_line - 2
                                && range.start.line <= expected_line + 2
                        })
                });
                assert!(
                    found,
                    "Expected caller '{expected_caller}' around line {expected_line} not found"
                );
            }
        }
        DaemonResponse::Error { error, .. } => {
            panic!("Request failed: {error}");
        }
        _ => panic!("Unexpected response type"),
    }

    Ok(())
}

async fn get_daemon_status(socket_path: &str) -> Result<DaemonStatus> {
    // Retry connection up to 3 times
    let mut stream = None;
    for attempt in 0..3 {
        match IpcStream::connect(socket_path).await {
            Ok(s) => {
                stream = Some(s);
                break;
            }
            Err(_e) if attempt < 2 => {
                sleep(Duration::from_millis(500)).await;
            }
            Err(e) => return Err(e),
        }
    }

    let mut stream = stream.unwrap();

    let request = DaemonRequest::Status {
        request_id: Uuid::new_v4(),
    };

    let encoded = MessageCodec::encode(&request)?;
    stream.write_all(&encoded).await?;

    let mut response_data = vec![0u8; 8192];
    let n = stream.read(&mut response_data).await?;
    response_data.truncate(n);

    match MessageCodec::decode_response(&response_data)? {
        DaemonResponse::Status { status, .. } => Ok(status),
        _ => panic!("Expected status response"),
    }
}

const GO_PROJECT1_CODE: &str = r#"
package main

import "fmt"

type DatabaseManager struct {
	host string
	port int
}

func NewDatabaseManager(host string, port int) *DatabaseManager {
	return &DatabaseManager{host: host, port: port}
}

func (dm *DatabaseManager) Connect() error {
	return connectToDatabase(dm.host, dm.port)
}

func connectToDatabase(host string, port int) error {
	fmt.Printf("Connecting to %s:%d\n", host, port)
	return nil
}

func main() {
	db := NewDatabaseManager("localhost", 5432)
	db.Connect()
	fmt.Println("Database operations completed")
}
"#;

const GO_PROJECT2_CODE: &str = r#"
package main

import "fmt"

type WebServer struct {
	port int
}

func NewWebServer(port int) *WebServer {
	return &WebServer{port: port}
}

func (ws *WebServer) Start() error {
	return startHTTPServer(ws.port)
}

func startHTTPServer(port int) error {
	fmt.Printf("Starting server on port %d\n", port)
	return nil
}

func main() {
	server := NewWebServer(8080)
	server.Start()
	fmt.Println("Web server operations completed")
}
"#;

const GO_PROJECT3_CODE: &str = r#"
package main

import "fmt"

type Calculator struct {
	history []string
}

func NewCalculator() *Calculator {
	return &Calculator{history: make([]string, 0)}
}

func (c *Calculator) Add(a, b float64) float64 {
	return performAddition(a, b)
}

func performAddition(a, b float64) float64 {
	return a + b
}

func main() {
	calc := NewCalculator()
	result := calc.Add(10, 5)
	fmt.Printf("10 + 5 = %.2f\n", result)
	fmt.Println("Calculator operations completed")
}
"#;

// Additional test for workspace isolation
#[tokio::test]
#[ignore = "Requires gopls with proper Go environment setup - run with --ignored to test"]
async fn test_workspace_isolation() -> Result<()> {
    // This test verifies that workspaces are properly isolated
    // and don't interfere with each other's symbol resolution

    // Clean up any existing daemon
    let _ = std::process::Command::new("pkill")
        .args(["-f", "lsp-daemon"])
        .output();

    sleep(Duration::from_millis(500)).await;

    let temp_dir = TempDir::new()?;

    // Create two projects with same function name but different implementations
    let workspace_a = setup_go_project(&temp_dir, "project_a", ISOLATION_PROJECT_A).await?;
    let workspace_b = setup_go_project(&temp_dir, "project_b", ISOLATION_PROJECT_B).await?;

    // Start daemon
    start_daemon_background().await?;
    sleep(Duration::from_millis(2000)).await; // Give more time for daemon to fully start

    let socket_path = get_default_socket_path();

    // Test that each workspace sees only its own functions
    test_project_analysis(&socket_path, &workspace_a, &[("main", 10)]).await?;
    test_project_analysis(&socket_path, &workspace_b, &[("main", 14)]).await?;

    println!("✅ Workspace isolation test completed successfully!");

    Ok(())
}

const ISOLATION_PROJECT_A: &str = r#"
package main

import "fmt"

func ProcessData() string {
    return "Processing in Project A"
}

func main() {
    result := ProcessData()
    fmt.Println(result)
}
"#;

const ISOLATION_PROJECT_B: &str = r#"
package main

import "fmt"

type DataProcessor struct{}

func (dp *DataProcessor) ProcessData() string {
    return "Processing in Project B"
}

func main() {
    dp := &DataProcessor{}
    result := dp.ProcessData()
    fmt.Println(result)
}
"#;

// Test for allowed_roots security constraint
#[tokio::test]
async fn test_allowed_roots_security() -> Result<()> {
    // This test would verify that the daemon respects allowed_roots constraints
    // when configured with restricted workspace access

    // Note: This would require extending the daemon startup to accept config
    // For now, we'll just verify the basic functionality works

    println!("✅ Security constraint test placeholder completed!");

    Ok(())
}

// Basic test to verify daemon starts and responds without requiring gopls
#[tokio::test]
#[ignore = "Daemon tests should run separately to avoid conflicts"]
async fn test_daemon_basic_functionality() -> Result<()> {
    // Clean up any existing daemon
    let _ = std::process::Command::new("pkill")
        .args(["-f", "lsp-daemon"])
        .output();

    sleep(Duration::from_millis(500)).await;

    // Start daemon
    start_daemon_background().await?;

    // Wait longer for daemon to be fully ready
    sleep(Duration::from_millis(3000)).await;

    let socket_path = get_default_socket_path();

    // Test basic connectivity and status with retry logic
    let mut status = None;
    for attempt in 0..5 {
        match get_daemon_status(&socket_path).await {
            Ok(s) => {
                status = Some(s);
                break;
            }
            Err(e) if attempt < 4 => {
                println!("Status attempt {} failed: {}, retrying...", attempt + 1, e);
                sleep(Duration::from_millis(1000)).await;
            }
            Err(e) => return Err(e),
        }
    }

    let status = status.expect("Failed to get daemon status after retries");

    // Verify daemon is running (basic sanity checks)
    // uptime_secs and total_requests are u64, so they're always >= 0

    println!("✅ Daemon basic functionality test passed!");
    println!("   - Uptime: {} seconds", status.uptime_secs);
    println!("   - Total pools: {}", status.pools.len());
    println!("   - Active connections: {}", status.active_connections);

    // Clean up daemon after test
    let _ = std::process::Command::new("pkill")
        .args(["-f", "lsp-daemon"])
        .output();

    Ok(())
}
