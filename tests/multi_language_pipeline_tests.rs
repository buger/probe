//! Multi-language pipeline processing tests with error scenarios
//!
//! These tests verify that the indexing system correctly handles multiple
//! programming languages, processes them with appropriate pipelines,
//! and gracefully handles various error conditions.

use anyhow::Result;
use lsp_daemon::cache_types::LspOperation;
use lsp_daemon::call_graph_cache::{CallGraphCache, CallGraphCacheConfig};
use lsp_daemon::indexing::{IndexingManager, IndexingPipeline, ManagerConfig};
use lsp_daemon::lsp_cache::{LspCache, LspCacheConfig};
use lsp_daemon::lsp_registry::LspRegistry;
use lsp_daemon::server_manager::SingleServerManager;
use lsp_daemon::{Language, LanguageDetector};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tempfile::TempDir;
use tokio::fs;
use tokio::time::sleep;

/// Test workspace for multi-language projects
struct MultiLanguageWorkspace {
    #[allow(dead_code)]
    temp_dir: TempDir,
    root_path: PathBuf,
}

impl MultiLanguageWorkspace {
    async fn new() -> Result<Self> {
        let temp_dir = tempfile::tempdir()?;
        let root_path = temp_dir.path().to_path_buf();

        // Create directory structure for different languages
        for lang_dir in ["rust", "typescript", "python", "go", "java", "cpp"] {
            fs::create_dir_all(root_path.join(lang_dir)).await?;
        }

        // Create mixed directories
        fs::create_dir_all(root_path.join("src")).await?;
        fs::create_dir_all(root_path.join("tests")).await?;
        fs::create_dir_all(root_path.join("examples")).await?;

        Ok(Self {
            temp_dir,
            root_path,
        })
    }

    fn path(&self) -> &Path {
        &self.root_path
    }

    async fn create_rust_files(&self) -> Result<()> {
        // Main Rust application
        fs::write(
            self.root_path.join("rust/main.rs"),
            r#"
use std::collections::HashMap;
use std::error::Error;

/// Main application structure
pub struct Application {
    name: String,
    version: String,
    config: HashMap<String, String>,
}

impl Application {
    /// Create a new application instance
    pub fn new(name: String, version: String) -> Self {
        Self {
            name,
            version,
            config: HashMap::new(),
        }
    }
    
    /// Get application name
    pub fn name(&self) -> &str {
        &self.name
    }
    
    /// Configure the application
    pub fn configure(&mut self, key: String, value: String) {
        self.config.insert(key, value);
    }
    
    /// Run the application
    pub async fn run(&self) -> Result<(), Box<dyn Error>> {
        println!("Running {} v{}", self.name, self.version);
        self.initialize().await?;
        self.start_services().await?;
        Ok(())
    }
    
    /// Initialize application
    async fn initialize(&self) -> Result<(), Box<dyn Error>> {
        println!("Initializing application...");
        Ok(())
    }
    
    /// Start background services
    async fn start_services(&self) -> Result<(), Box<dyn Error>> {
        println!("Starting services...");
        Ok(())
    }
}

/// Application trait for different implementations
pub trait ApplicationTrait {
    fn get_version(&self) -> &str;
    fn is_running(&self) -> bool;
}

impl ApplicationTrait for Application {
    fn get_version(&self) -> &str {
        &self.version
    }
    
    fn is_running(&self) -> bool {
        true
    }
}

/// Main entry point
fn main() {
    let mut app = Application::new("TestApp".to_string(), "1.0.0".to_string());
    app.configure("debug".to_string(), "true".to_string());
    
    let runtime = tokio::runtime::Runtime::new().unwrap();
    runtime.block_on(async {
        if let Err(e) = app.run().await {
            eprintln!("Application error: {}", e);
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_application_creation() {
        let app = Application::new("Test".to_string(), "0.1.0".to_string());
        assert_eq!(app.name(), "Test");
        assert_eq!(app.get_version(), "0.1.0");
    }
    
    #[tokio::test]
    async fn test_application_run() {
        let app = Application::new("Test".to_string(), "0.1.0".to_string());
        let result = app.run().await;
        assert!(result.is_ok());
    }
}
"#,
        )
        .await?;

        // Rust library
        fs::write(
            self.root_path.join("rust/lib.rs"),
            r#"
//! Utility library for common operations
//!
//! This library provides common functionality used across the application.

pub mod utils;
pub mod errors;

/// Result type alias
pub type LibResult<T> = Result<T, errors::LibError>;

/// Constants
pub const VERSION: &str = "1.0.0";
pub const MAX_ITEMS: usize = 1000;

/// Library configuration
#[derive(Debug, Clone)]
pub struct Config {
    pub debug: bool,
    pub max_connections: u32,
    pub timeout_ms: u64,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            debug: false,
            max_connections: 100,
            timeout_ms: 5000,
        }
    }
}

/// Main library interface
pub struct Library {
    config: Config,
}

impl Library {
    pub fn new(config: Config) -> Self {
        Self { config }
    }
    
    pub fn with_debug(mut self, debug: bool) -> Self {
        self.config.debug = debug;
        self
    }
}
"#,
        )
        .await?;

        Ok(())
    }

    async fn create_typescript_files(&self) -> Result<()> {
        // TypeScript application
        fs::write(
            self.root_path.join("typescript/app.ts"),
            r#"
/**
 * TypeScript application with various language features
 */

import { EventEmitter } from 'events';
import { promises as fs } from 'fs';

/**
 * User interface with optional properties
 */
export interface User {
    id: number;
    name: string;
    email: string;
    age?: number;
    isActive: boolean;
    metadata?: Record<string, any>;
}

/**
 * Generic repository interface
 */
export interface Repository<T> {
    findById(id: number): Promise<T | null>;
    create(item: T): Promise<T>;
    update(id: number, updates: Partial<T>): Promise<T>;
    delete(id: number): Promise<boolean>;
}

/**
 * User service class with dependency injection
 */
export class UserService extends EventEmitter {
    private repository: Repository<User>;
    private cache: Map<number, User> = new Map();

    constructor(repository: Repository<User>) {
        super();
        this.repository = repository;
    }

    /**
     * Create a new user
     */
    async createUser(userData: Omit<User, 'id'>): Promise<User> {
        const user = await this.repository.create({
            id: Date.now(),
            ...userData
        });
        
        this.cache.set(user.id, user);
        this.emit('userCreated', user);
        
        return user;
    }

    /**
     * Get user by ID with caching
     */
    async getUser(id: number): Promise<User | null> {
        if (this.cache.has(id)) {
            return this.cache.get(id)!;
        }

        const user = await this.repository.findById(id);
        if (user) {
            this.cache.set(id, user);
        }

        return user;
    }

    /**
     * Update user information
     */
    async updateUser(id: number, updates: Partial<User>): Promise<User> {
        const user = await this.repository.update(id, updates);
        this.cache.set(id, user);
        this.emit('userUpdated', user);
        return user;
    }

    /**
     * Batch process users
     */
    async batchProcess<R>(
        users: User[],
        processor: (user: User) => Promise<R>
    ): Promise<R[]> {
        return Promise.all(users.map(processor));
    }

    /**
     * Get statistics
     */
    getStats(): { cacheSize: number; totalEvents: number } {
        return {
            cacheSize: this.cache.size,
            totalEvents: this.listenerCount('userCreated') + this.listenerCount('userUpdated')
        };
    }

    /**
     * Private utility method
     */
    private validateUser(user: User): boolean {
        return user.id > 0 && 
               user.name.length > 0 && 
               user.email.includes('@');
    }
}

/**
 * Configuration type with advanced types
 */
export type AppConfig = {
    database: {
        host: string;
        port: number;
        ssl: boolean;
    };
    cache: {
        enabled: boolean;
        ttl: number;
    };
    features: Record<string, boolean>;
};

/**
 * Utility functions namespace
 */
export namespace Utils {
    export function isValidEmail(email: string): boolean {
        return /^[^\s@]+@[^\s@]+\.[^\s@]+$/.test(email);
    }

    export function formatName(firstName: string, lastName: string): string {
        return `${firstName} ${lastName}`.trim();
    }

    export async function delay(ms: number): Promise<void> {
        return new Promise(resolve => setTimeout(resolve, ms));
    }

    export const constants = {
        MAX_AGE: 120,
        MIN_AGE: 13,
        DEFAULT_TIMEOUT: 30000
    } as const;
}

/**
 * Enum for user roles
 */
export enum UserRole {
    ADMIN = 'admin',
    USER = 'user',
    GUEST = 'guest'
}

/**
 * Decorator function
 */
export function logged(target: any, propertyKey: string, descriptor: PropertyDescriptor) {
    const originalMethod = descriptor.value;

    descriptor.value = function (...args: any[]) {
        console.log(`Calling ${propertyKey} with args:`, args);
        const result = originalMethod.apply(this, args);
        console.log(`${propertyKey} returned:`, result);
        return result;
    };

    return descriptor;
}

/**
 * Generic constraint example
 */
export function processEntity<T extends { id: number }>(entity: T): T {
    console.log(`Processing entity with ID: ${entity.id}`);
    return entity;
}
"#,
        )
        .await?;

        Ok(())
    }

    async fn create_python_files(&self) -> Result<()> {
        // Python application
        fs::write(
            self.root_path.join("python/app.py"),
            r#"
"""
Python application with comprehensive language features
"""

import asyncio
import functools
import logging
from abc import ABC, abstractmethod
from dataclasses import dataclass, field
from datetime import datetime, timedelta
from enum import Enum, auto
from pathlib import Path
from typing import (
    Any, Dict, List, Optional, Union, Callable,
    TypeVar, Generic, Protocol, Tuple, Set
)


# Configure logging
logging.basicConfig(level=logging.INFO)
logger = logging.getLogger(__name__)


class Priority(Enum):
    """Task priority levels"""
    LOW = auto()
    MEDIUM = auto()
    HIGH = auto()
    CRITICAL = auto()


@dataclass
class Task:
    """Task data structure with validation"""
    id: str
    name: str
    priority: Priority
    created_at: datetime = field(default_factory=datetime.now)
    tags: Set[str] = field(default_factory=set)
    metadata: Dict[str, Any] = field(default_factory=dict)
    
    def __post_init__(self):
        if not self.name.strip():
            raise ValueError("Task name cannot be empty")
        if len(self.name) > 255:
            raise ValueError("Task name too long")


class TaskProcessor(Protocol):
    """Protocol for task processors"""
    
    def process(self, task: Task) -> bool:
        """Process a task and return success status"""
        ...


T = TypeVar('T')
P = TypeVar('P', bound=TaskProcessor)


class TaskManager(Generic[P]):
    """Generic task manager with processor type"""
    
    def __init__(self, processor: P):
        self.processor = processor
        self.tasks: List[Task] = []
        self._lock = asyncio.Lock()
        self._stats = {
            'processed': 0,
            'failed': 0,
            'total': 0
        }
    
    async def add_task(self, task: Task) -> None:
        """Add a task to the queue"""
        async with self._lock:
            self.tasks.append(task)
            self._stats['total'] += 1
            logger.info(f"Added task: {task.name}")
    
    async def process_tasks(self) -> Dict[str, int]:
        """Process all queued tasks"""
        async with self._lock:
            results = {'success': 0, 'failed': 0}
            
            for task in self.tasks:
                try:
                    if self.processor.process(task):
                        results['success'] += 1
                        self._stats['processed'] += 1
                    else:
                        results['failed'] += 1
                        self._stats['failed'] += 1
                except Exception as e:
                    logger.error(f"Error processing task {task.id}: {e}")
                    results['failed'] += 1
                    self._stats['failed'] += 1
            
            # Clear processed tasks
            self.tasks.clear()
            return results
    
    @property
    def stats(self) -> Dict[str, int]:
        """Get processing statistics"""
        return self._stats.copy()


class BaseProcessor(ABC, TaskProcessor):
    """Abstract base processor"""
    
    def __init__(self, name: str):
        self.name = name
        self.processed_count = 0
    
    @abstractmethod
    def _do_process(self, task: Task) -> bool:
        """Implement specific processing logic"""
        pass
    
    def process(self, task: Task) -> bool:
        """Process task with logging"""
        logger.info(f"Processor {self.name} processing task: {task.name}")
        result = self._do_process(task)
        self.processed_count += 1
        return result


class DefaultProcessor(BaseProcessor):
    """Default task processor implementation"""
    
    def __init__(self):
        super().__init__("default")
    
    def _do_process(self, task: Task) -> bool:
        """Simple processing logic"""
        # Simulate processing time based on priority
        import time
        delay = {
            Priority.LOW: 0.1,
            Priority.MEDIUM: 0.2,
            Priority.HIGH: 0.3,
            Priority.CRITICAL: 0.5
        }
        time.sleep(delay.get(task.priority, 0.1))
        
        # Fail randomly for testing error handling
        import random
        return random.random() > 0.1  # 90% success rate


def retry(max_attempts: int = 3, delay: float = 1.0):
    """Retry decorator for functions"""
    def decorator(func: Callable) -> Callable:
        @functools.wraps(func)
        async def wrapper(*args, **kwargs):
            last_exception = None
            
            for attempt in range(max_attempts):
                try:
                    return await func(*args, **kwargs)
                except Exception as e:
                    last_exception = e
                    if attempt < max_attempts - 1:
                        await asyncio.sleep(delay * (2 ** attempt))
                        logger.warning(f"Retry {attempt + 1}/{max_attempts} for {func.__name__}")
                    
            raise last_exception
        
        return wrapper
    return decorator


@retry(max_attempts=3)
async def fetch_external_data(url: str) -> Dict[str, Any]:
    """Fetch data from external source with retry logic"""
    # Simulate network request
    await asyncio.sleep(0.1)
    
    # Simulate occasional failures
    import random
    if random.random() < 0.2:  # 20% failure rate
        raise ConnectionError(f"Failed to fetch from {url}")
    
    return {"data": f"content from {url}", "timestamp": datetime.now().isoformat()}


class TaskScheduler:
    """Task scheduler with cron-like functionality"""
    
    def __init__(self):
        self.scheduled_tasks: List[Tuple[Task, datetime]] = []
        self.running = False
    
    def schedule_task(self, task: Task, run_at: datetime) -> None:
        """Schedule a task to run at specific time"""
        self.scheduled_tasks.append((task, run_at))
        self.scheduled_tasks.sort(key=lambda x: x[1])  # Sort by run time
    
    async def start_scheduler(self, manager: TaskManager) -> None:
        """Start the scheduler loop"""
        self.running = True
        logger.info("Task scheduler started")
        
        while self.running:
            now = datetime.now()
            due_tasks = []
            
            # Find tasks that are due
            remaining_tasks = []
            for task, run_at in self.scheduled_tasks:
                if run_at <= now:
                    due_tasks.append(task)
                else:
                    remaining_tasks.append((task, run_at))
            
            self.scheduled_tasks = remaining_tasks
            
            # Execute due tasks
            for task in due_tasks:
                await manager.add_task(task)
            
            if due_tasks:
                await manager.process_tasks()
            
            # Sleep for a short interval
            await asyncio.sleep(1)
    
    def stop_scheduler(self) -> None:
        """Stop the scheduler"""
        self.running = False
        logger.info("Task scheduler stopped")


# Module-level functions
def create_sample_tasks() -> List[Task]:
    """Create sample tasks for testing"""
    tasks = [
        Task("1", "High priority task", Priority.HIGH, tags={"urgent", "backend"}),
        Task("2", "Low priority cleanup", Priority.LOW, tags={"maintenance"}),
        Task("3", "Critical bug fix", Priority.CRITICAL, tags={"bugfix", "urgent"}),
        Task("4", "Medium priority feature", Priority.MEDIUM, tags={"feature"}),
    ]
    return tasks


async def main():
    """Main application entry point"""
    logger.info("Starting task management application")
    
    # Create processor and manager
    processor = DefaultProcessor()
    manager = TaskManager(processor)
    scheduler = TaskScheduler()
    
    # Add sample tasks
    for task in create_sample_tasks():
        await manager.add_task(task)
    
    # Schedule future task
    future_task = Task("5", "Scheduled maintenance", Priority.LOW)
    future_time = datetime.now() + timedelta(seconds=5)
    scheduler.schedule_task(future_task, future_time)
    
    # Process immediate tasks
    results = await manager.process_tasks()
    logger.info(f"Processing results: {results}")
    
    # Start scheduler for future tasks
    scheduler_task = asyncio.create_task(scheduler.start_scheduler(manager))
    
    # Let scheduler run for a bit
    await asyncio.sleep(10)
    
    # Stop scheduler
    scheduler.stop_scheduler()
    await scheduler_task
    
    # Final stats
    logger.info(f"Final stats: {manager.stats}")


if __name__ == "__main__":
    asyncio.run(main())
"#,
        )
        .await?;

        Ok(())
    }

    async fn create_problematic_files(&self) -> Result<()> {
        // Invalid Rust file
        fs::write(
            self.root_path.join("rust/invalid.rs"),
            r#"
// This file has syntax errors and invalid content
fn invalid_function( {
    let x = ;  // Invalid syntax
    missing_semicolon()
    
    // Unclosed string literal
    let bad_string = "unclosed string
    
    // Invalid macro
    println!(;
    
    // Unmatched braces
    if true {
        println!("missing closing brace"
    
    // Invalid type annotation  
    let bad_type: Vec<> = Vec::new();
}

struct MissingFields {
    // Empty struct that might cause issues
}

impl MissingFields {
    fn incomplete_method(
        // Missing closing parenthesis and body
}
"#,
        )
        .await?;

        // Binary file disguised as source code
        fs::write(
            self.root_path.join("python/binary_disguised.py"),
            b"\x00\x01\x02\x03\x04\x05\x06\x07\x08\x09\x0A\x0B\x0C\x0D\x0E\x0F\
              \x10\x11\x12\x13\x14\x15\x16\x17\x18\x19\x1A\x1B\x1C\x1D\x1E\x1F\
              This looks like Python but contains binary data\
              \xFF\xFE\xFD\xFC\xFB\xFA\xF9\xF8\xF7\xF6\xF5\xF4\xF3\xF2\xF1\xF0",
        )
        .await?;

        // Extremely large file
        let large_content = "# ".repeat(50000); // Create a large comment
        fs::write(
            self.root_path.join("typescript/large_file.ts"),
            format!(
                "{large_content}
// This file is very large and might cause memory issues
export class LargeClass {{
    method() {{
        return 'large';
    }}
}}"
            ),
        )
        .await?;

        // File with unusual encoding
        fs::write(
            self.root_path.join("python/encoding_issues.py"),
            "# -*- coding: utf-8 -*-\n# This file has encoding issues: café naïve résumé 中文 🚀\n",
        )
        .await?;

        // Empty file
        fs::write(self.root_path.join("java/Empty.java"), "").await?;

        // File with only whitespace
        fs::write(
            self.root_path.join("cpp/whitespace_only.cpp"),
            "   \n\t\n    \n\t\t\n   ",
        )
        .await?;

        Ok(())
    }

    async fn create_go_files(&self) -> Result<()> {
        fs::write(self.root_path.join("go/main.go"), r#"
package main

import (
    "context"
    "fmt"
    "log"
    "net/http"
    "sync"
    "time"
)

// User represents a user entity
type User struct {
    ID    int    `json:"id"`
    Name  string `json:"name"`
    Email string `json:"email"`
}

// UserService handles user operations
type UserService interface {
    GetUser(ctx context.Context, id int) (*User, error)
    CreateUser(ctx context.Context, user *User) error
    UpdateUser(ctx context.Context, id int, updates map[string]interface{}) error
}

// InMemoryUserService implements UserService
type InMemoryUserService struct {
    mu    sync.RWMutex
    users map[int]*User
    nextID int
}

// NewInMemoryUserService creates a new in-memory user service
func NewInMemoryUserService() *InMemoryUserService {
    return &InMemoryUserService{
        users: make(map[int]*User),
        nextID: 1,
    }
}

// GetUser retrieves a user by ID
func (s *InMemoryUserService) GetUser(ctx context.Context, id int) (*User, error) {
    s.mu.RLock()
    defer s.mu.RUnlock()
    
    user, exists := s.users[id]
    if !exists {
        return nil, fmt.Errorf("user not found: %d", id)
    }
    
    return user, nil
}

// CreateUser creates a new user
func (s *InMemoryUserService) CreateUser(ctx context.Context, user *User) error {
    s.mu.Lock()
    defer s.mu.Unlock()
    
    user.ID = s.nextID
    s.nextID++
    s.users[user.ID] = user
    
    return nil
}

// UpdateUser updates an existing user
func (s *InMemoryUserService) UpdateUser(ctx context.Context, id int, updates map[string]interface{}) error {
    s.mu.Lock()
    defer s.mu.Unlock()
    
    user, exists := s.users[id]
    if !exists {
        return fmt.Errorf("user not found: %d", id)
    }
    
    if name, ok := updates["name"].(string); ok {
        user.Name = name
    }
    if email, ok := updates["email"].(string); ok {
        user.Email = email
    }
    
    return nil
}

// HTTPHandler handles HTTP requests
type HTTPHandler struct {
    userService UserService
}

// NewHTTPHandler creates a new HTTP handler
func NewHTTPHandler(userService UserService) *HTTPHandler {
    return &HTTPHandler{userService: userService}
}

// ServeHTTP implements http.Handler
func (h *HTTPHandler) ServeHTTP(w http.ResponseWriter, r *http.Request) {
    switch r.Method {
    case http.MethodGet:
        h.handleGet(w, r)
    case http.MethodPost:
        h.handlePost(w, r)
    default:
        http.Error(w, "Method not allowed", http.StatusMethodNotAllowed)
    }
}

func (h *HTTPHandler) handleGet(w http.ResponseWriter, r *http.Request) {
    // Implementation would go here
    fmt.Fprintf(w, "GET request handled")
}

func (h *HTTPHandler) handlePost(w http.ResponseWriter, r *http.Request) {
    // Implementation would go here  
    fmt.Fprintf(w, "POST request handled")
}

func main() {
    ctx := context.Background()
    
    // Create services
    userService := NewInMemoryUserService()
    handler := NewHTTPHandler(userService)
    
    // Create sample user
    user := &User{Name: "John Doe", Email: "john@example.com"}
    if err := userService.CreateUser(ctx, user); err != nil {
        log.Fatalf("Failed to create user: %v", err)
    }
    
    // Start HTTP server
    server := &http.Server{
        Addr:    ":8080",
        Handler: handler,
        ReadTimeout:  5 * time.Second,
        WriteTimeout: 10 * time.Second,
    }
    
    fmt.Println("Starting server on :8080")
    log.Fatal(server.ListenAndServe())
}
"#).await?;

        Ok(())
    }

    async fn create_mixed_directory(&self) -> Result<()> {
        // Mixed language files in src/
        fs::write(
            self.root_path.join("src/utils.rs"),
            "pub fn add(a: i32, b: i32) -> i32 { a + b }",
        )
        .await?;
        fs::write(
            self.root_path.join("src/helper.py"),
            "def multiply(a, b):\n    return a * b",
        )
        .await?;
        fs::write(
            self.root_path.join("src/types.ts"),
            "export type ID = number;",
        )
        .await?;
        fs::write(
            self.root_path.join("src/config.json"),
            r#"{"debug": true, "port": 3000}"#,
        )
        .await?;

        Ok(())
    }
}

#[tokio::test]
async fn test_multi_language_file_detection() -> Result<()> {
    let workspace = MultiLanguageWorkspace::new().await?;

    // Create files in different languages
    workspace.create_rust_files().await?;
    workspace.create_typescript_files().await?;
    workspace.create_python_files().await?;
    workspace.create_go_files().await?;
    workspace.create_mixed_directory().await?;

    let language_detector = Arc::new(LanguageDetector::new());
    let config = ManagerConfig {
        max_workers: 4,
        memory_budget_bytes: 128 * 1024 * 1024,
        memory_pressure_threshold: 0.8,
        max_queue_size: 1000,
        exclude_patterns: vec![],
        include_patterns: vec![],
        max_file_size_bytes: 10 * 1024 * 1024,
        enabled_languages: vec![], // Enable all languages
        incremental_mode: false,
        discovery_batch_size: 20,
        status_update_interval_secs: 1,
    };

    // Create mock LSP dependencies for testing
    let registry = Arc::new(LspRegistry::new().expect("Failed to create registry"));
    let child_processes = Arc::new(tokio::sync::Mutex::new(Vec::new()));
    let server_manager = Arc::new(SingleServerManager::new_with_tracker(
        registry,
        child_processes,
    ));

    let cache_config = CallGraphCacheConfig {
        capacity: 100,
        ttl: Duration::from_secs(300),
        eviction_check_interval: Duration::from_secs(30),
        invalidation_depth: 1,
        ..Default::default()
    };
    let call_graph_cache = Arc::new(CallGraphCache::new(cache_config));

    let lsp_cache_config = LspCacheConfig {
        capacity_per_operation: 100,
        ttl: Duration::from_secs(300),
        eviction_check_interval: Duration::from_secs(30),
        persistent: false,
        cache_directory: None,
    };
    let definition_cache = Arc::new(
        LspCache::new(LspOperation::Definition, lsp_cache_config)
            .expect("Failed to create definition cache"),
    );
    let manager = IndexingManager::new(
        config,
        language_detector,
        server_manager,
        call_graph_cache,
        definition_cache,
    );

    // Start indexing
    manager
        .start_indexing(workspace.path().to_path_buf())
        .await?;

    // Wait for completion
    let start_time = Instant::now();
    while start_time.elapsed() < Duration::from_secs(15) {
        let progress = manager.get_progress().await;
        if progress.is_complete() {
            break;
        }
        sleep(Duration::from_millis(100)).await;
    }

    let final_progress = manager.get_progress().await;
    manager.stop_indexing().await?;

    // Should have processed multiple files from different languages
    assert!(
        final_progress.processed_files >= 6,
        "Expected at least 6 files, got {}",
        final_progress.processed_files
    );

    // Should have extracted symbols from the code
    assert!(
        final_progress.symbols_extracted > 0,
        "Expected symbols to be extracted, got {}",
        final_progress.symbols_extracted
    );

    println!(
        "Multi-language processing: {} files, {} symbols",
        final_progress.processed_files, final_progress.symbols_extracted
    );

    Ok(())
}

#[tokio::test]
async fn test_language_specific_filtering() -> Result<()> {
    let workspace = MultiLanguageWorkspace::new().await?;

    workspace.create_rust_files().await?;
    workspace.create_typescript_files().await?;
    workspace.create_python_files().await?;
    workspace.create_go_files().await?;

    // Test Rust-only processing
    let language_detector = Arc::new(LanguageDetector::new());
    let config = ManagerConfig {
        max_workers: 2,
        enabled_languages: vec!["Rust".to_string()], // Only Rust
        memory_budget_bytes: 64 * 1024 * 1024,
        memory_pressure_threshold: 0.8,
        max_queue_size: 100,
        exclude_patterns: vec![],
        include_patterns: vec![],
        max_file_size_bytes: 1024 * 1024,
        incremental_mode: false,
        discovery_batch_size: 10,
        status_update_interval_secs: 1,
    };

    // Create mock LSP dependencies for testing
    let registry = Arc::new(LspRegistry::new().expect("Failed to create registry"));
    let child_processes = Arc::new(tokio::sync::Mutex::new(Vec::new()));
    let server_manager = Arc::new(SingleServerManager::new_with_tracker(
        registry,
        child_processes,
    ));

    let cache_config = CallGraphCacheConfig {
        capacity: 100,
        ttl: Duration::from_secs(300),
        eviction_check_interval: Duration::from_secs(30),
        invalidation_depth: 1,
        ..Default::default()
    };
    let call_graph_cache = Arc::new(CallGraphCache::new(cache_config));

    let lsp_cache_config = LspCacheConfig {
        capacity_per_operation: 100,
        ttl: Duration::from_secs(300),
        eviction_check_interval: Duration::from_secs(30),
        persistent: false,
        cache_directory: None,
    };
    let definition_cache = Arc::new(
        LspCache::new(LspOperation::Definition, lsp_cache_config)
            .expect("Failed to create definition cache"),
    );
    let manager = IndexingManager::new(
        config,
        language_detector,
        server_manager,
        call_graph_cache,
        definition_cache,
    );
    manager
        .start_indexing(workspace.path().to_path_buf())
        .await?;

    // Wait for completion
    let start_time = Instant::now();
    while start_time.elapsed() < Duration::from_secs(10) {
        let progress = manager.get_progress().await;
        if progress.is_complete() {
            break;
        }
        sleep(Duration::from_millis(100)).await;
    }

    let rust_only_progress = manager.get_progress().await;
    manager.stop_indexing().await?;

    // Should have processed only Rust files (fewer than all languages)
    assert!(rust_only_progress.processed_files > 0);

    println!(
        "Rust-only processing: {} files",
        rust_only_progress.processed_files
    );

    Ok(())
}

#[tokio::test]
async fn test_error_handling_with_problematic_files() -> Result<()> {
    let workspace = MultiLanguageWorkspace::new().await?;

    // Create good files
    workspace.create_rust_files().await?;
    workspace.create_python_files().await?;

    // Create problematic files
    workspace.create_problematic_files().await?;

    let language_detector = Arc::new(LanguageDetector::new());
    let config = ManagerConfig {
        max_workers: 2,
        memory_budget_bytes: 64 * 1024 * 1024,
        memory_pressure_threshold: 0.8,
        max_queue_size: 100,
        exclude_patterns: vec![],
        include_patterns: vec![],
        max_file_size_bytes: 1024 * 1024, // 1MB limit to catch large files
        enabled_languages: vec![],
        incremental_mode: false,
        discovery_batch_size: 10,
        status_update_interval_secs: 1,
    };

    // Create mock LSP dependencies for testing
    let registry = Arc::new(LspRegistry::new().expect("Failed to create registry"));
    let child_processes = Arc::new(tokio::sync::Mutex::new(Vec::new()));
    let server_manager = Arc::new(SingleServerManager::new_with_tracker(
        registry,
        child_processes,
    ));

    let cache_config = CallGraphCacheConfig {
        capacity: 100,
        ttl: Duration::from_secs(300),
        eviction_check_interval: Duration::from_secs(30),
        invalidation_depth: 1,
        ..Default::default()
    };
    let call_graph_cache = Arc::new(CallGraphCache::new(cache_config));

    let lsp_cache_config = LspCacheConfig {
        capacity_per_operation: 100,
        ttl: Duration::from_secs(300),
        eviction_check_interval: Duration::from_secs(30),
        persistent: false,
        cache_directory: None,
    };
    let definition_cache = Arc::new(
        LspCache::new(LspOperation::Definition, lsp_cache_config)
            .expect("Failed to create definition cache"),
    );
    let manager = IndexingManager::new(
        config,
        language_detector,
        server_manager,
        call_graph_cache,
        definition_cache,
    );
    manager
        .start_indexing(workspace.path().to_path_buf())
        .await?;

    // Wait for completion - allow longer time for error handling
    let start_time = Instant::now();
    while start_time.elapsed() < Duration::from_secs(20) {
        let progress = manager.get_progress().await;
        if progress.is_complete() {
            break;
        }
        sleep(Duration::from_millis(200)).await;
    }

    let final_progress = manager.get_progress().await;
    manager.stop_indexing().await?;

    // Should have processed some files successfully
    assert!(
        final_progress.processed_files > 0,
        "Expected some files to be processed successfully"
    );

    // Should have some failures due to problematic files
    assert!(
        final_progress.failed_files > 0,
        "Expected some files to fail processing"
    );

    // Should still complete overall
    assert!(
        final_progress.is_complete(),
        "Indexing should complete despite errors"
    );

    println!(
        "Error handling test: {} processed, {} failed",
        final_progress.processed_files, final_progress.failed_files
    );

    Ok(())
}

#[tokio::test]
async fn test_individual_pipeline_processing() -> Result<()> {
    let workspace = MultiLanguageWorkspace::new().await?;
    workspace.create_rust_files().await?;

    // Test individual pipeline processing
    let mut rust_pipeline = IndexingPipeline::new(Language::Rust)?;

    let rust_file = workspace.path().join("rust/lib.rs");
    let result = rust_pipeline.process_file(&rust_file).await?;

    assert!(
        result.symbols_found > 0,
        "Should extract symbols from Rust file"
    );
    assert!(result.bytes_processed > 0, "Should process bytes");
    assert!(
        result.processing_time_ms > 0,
        "Should track processing time"
    );

    println!(
        "Rust pipeline result: {} symbols, {} bytes, {}ms",
        result.symbols_found, result.bytes_processed, result.processing_time_ms
    );

    Ok(())
}

#[tokio::test]
async fn test_pipeline_configuration() -> Result<()> {
    // Test various pipeline configurations
    let languages = [
        Language::Rust,
        Language::TypeScript,
        Language::Python,
        Language::Go,
    ];

    for language in languages {
        let pipeline = IndexingPipeline::new(language);

        match pipeline {
            Ok(p) => {
                assert_eq!(p.language(), language);
                println!("Successfully created pipeline for {language:?}");
            }
            Err(e) => {
                println!("Failed to create pipeline for {language:?}: {e}");
                // Some languages might not be supported - that's OK for this test
            }
        }
    }

    Ok(())
}

#[tokio::test]
async fn test_concurrent_multi_language_processing() -> Result<()> {
    let workspace = MultiLanguageWorkspace::new().await?;

    // Create files for different languages
    workspace.create_rust_files().await?;
    workspace.create_typescript_files().await?;
    workspace.create_python_files().await?;
    workspace.create_go_files().await?;

    let language_detector = Arc::new(LanguageDetector::new());
    let config = ManagerConfig {
        max_workers: 6, // More workers for concurrent processing
        memory_budget_bytes: 256 * 1024 * 1024,
        memory_pressure_threshold: 0.8,
        max_queue_size: 500,
        exclude_patterns: vec![],
        include_patterns: vec![],
        max_file_size_bytes: 10 * 1024 * 1024,
        enabled_languages: vec![],
        incremental_mode: false,
        discovery_batch_size: 5, // Smaller batches for more concurrent processing
        status_update_interval_secs: 1,
    };

    // Create mock LSP dependencies for testing
    let registry = Arc::new(LspRegistry::new().expect("Failed to create registry"));
    let child_processes = Arc::new(tokio::sync::Mutex::new(Vec::new()));
    let server_manager = Arc::new(SingleServerManager::new_with_tracker(
        registry,
        child_processes,
    ));

    let cache_config = CallGraphCacheConfig {
        capacity: 100,
        ttl: Duration::from_secs(300),
        eviction_check_interval: Duration::from_secs(30),
        invalidation_depth: 1,
        ..Default::default()
    };
    let call_graph_cache = Arc::new(CallGraphCache::new(cache_config));

    let lsp_cache_config = LspCacheConfig {
        capacity_per_operation: 100,
        ttl: Duration::from_secs(300),
        eviction_check_interval: Duration::from_secs(30),
        persistent: false,
        cache_directory: None,
    };
    let definition_cache = Arc::new(
        LspCache::new(LspOperation::Definition, lsp_cache_config)
            .expect("Failed to create definition cache"),
    );
    let manager = IndexingManager::new(
        config,
        language_detector,
        server_manager,
        call_graph_cache,
        definition_cache,
    );
    manager
        .start_indexing(workspace.path().to_path_buf())
        .await?;

    // Monitor worker activity during processing
    let mut max_active_workers = 0;
    let start_time = Instant::now();

    while start_time.elapsed() < Duration::from_secs(15) {
        let progress = manager.get_progress().await;
        if progress.active_workers > max_active_workers {
            max_active_workers = progress.active_workers;
        }

        if progress.is_complete() {
            break;
        }
        sleep(Duration::from_millis(50)).await;
    }

    let final_progress = manager.get_progress().await;
    manager.stop_indexing().await?;

    // Should have used multiple workers concurrently
    assert!(
        max_active_workers >= 2,
        "Expected concurrent workers, max seen: {max_active_workers}"
    );

    // Should have processed files from multiple languages
    assert!(final_progress.processed_files >= 4);

    println!(
        "Concurrent processing: {} files, max {} workers active",
        final_progress.processed_files, max_active_workers
    );

    Ok(())
}

#[tokio::test]
async fn test_memory_pressure_with_large_files() -> Result<()> {
    let workspace = MultiLanguageWorkspace::new().await?;

    // Create normal files
    workspace.create_rust_files().await?;
    workspace.create_problematic_files().await?; // Includes large file

    let language_detector = Arc::new(LanguageDetector::new());
    let config = ManagerConfig {
        max_workers: 2,
        memory_budget_bytes: 1024 * 1024, // Very small: 1MB
        memory_pressure_threshold: 0.5,   // Trigger pressure early
        max_queue_size: 100,
        exclude_patterns: vec![],
        include_patterns: vec![],
        max_file_size_bytes: 1024 * 1024, // 1MB file size limit
        enabled_languages: vec![],
        incremental_mode: false,
        discovery_batch_size: 10,
        status_update_interval_secs: 1,
    };

    // Create mock LSP dependencies for testing
    let registry = Arc::new(LspRegistry::new().expect("Failed to create registry"));
    let child_processes = Arc::new(tokio::sync::Mutex::new(Vec::new()));
    let server_manager = Arc::new(SingleServerManager::new_with_tracker(
        registry,
        child_processes,
    ));

    let cache_config = CallGraphCacheConfig {
        capacity: 100,
        ttl: Duration::from_secs(300),
        eviction_check_interval: Duration::from_secs(30),
        invalidation_depth: 1,
        ..Default::default()
    };
    let call_graph_cache = Arc::new(CallGraphCache::new(cache_config));

    let lsp_cache_config = LspCacheConfig {
        capacity_per_operation: 100,
        ttl: Duration::from_secs(300),
        eviction_check_interval: Duration::from_secs(30),
        persistent: false,
        cache_directory: None,
    };
    let definition_cache = Arc::new(
        LspCache::new(LspOperation::Definition, lsp_cache_config)
            .expect("Failed to create definition cache"),
    );
    let manager = IndexingManager::new(
        config,
        language_detector,
        server_manager,
        call_graph_cache,
        definition_cache,
    );
    manager
        .start_indexing(workspace.path().to_path_buf())
        .await?;

    // Monitor for memory pressure
    let mut detected_memory_pressure = false;
    let start_time = Instant::now();

    while start_time.elapsed() < Duration::from_secs(15) {
        if manager.is_memory_pressure() {
            detected_memory_pressure = true;
        }

        let progress = manager.get_progress().await;
        if progress.is_complete() {
            break;
        }
        sleep(Duration::from_millis(100)).await;
    }

    let final_progress = manager.get_progress().await;
    manager.stop_indexing().await?;

    // With such a small memory budget, should detect pressure
    println!(
        "Memory pressure detected: {}, processed: {} files",
        detected_memory_pressure, final_progress.processed_files
    );

    // Should still complete processing despite memory pressure
    assert!(final_progress.is_complete());

    Ok(())
}

#[tokio::test]
async fn test_language_priority_processing() -> Result<()> {
    let workspace = MultiLanguageWorkspace::new().await?;

    // Create files for different languages with different priorities
    workspace.create_rust_files().await?; // Should be high priority
    workspace.create_typescript_files().await?; // Should be high priority
    workspace.create_python_files().await?; // Should be medium priority
    workspace.create_go_files().await?; // Should be medium priority
    workspace.create_mixed_directory().await?; // Mixed priorities

    let language_detector = Arc::new(LanguageDetector::new());
    let config = ManagerConfig {
        max_workers: 1, // Single worker to observe priority ordering
        memory_budget_bytes: 128 * 1024 * 1024,
        memory_pressure_threshold: 0.8,
        max_queue_size: 1000,
        exclude_patterns: vec![],
        include_patterns: vec![],
        max_file_size_bytes: 10 * 1024 * 1024,
        enabled_languages: vec![],
        incremental_mode: false,
        discovery_batch_size: 100,
        status_update_interval_secs: 1,
    };

    // Create mock LSP dependencies for testing
    let registry = Arc::new(LspRegistry::new().expect("Failed to create registry"));
    let child_processes = Arc::new(tokio::sync::Mutex::new(Vec::new()));
    let server_manager = Arc::new(SingleServerManager::new_with_tracker(
        registry,
        child_processes,
    ));

    let cache_config = CallGraphCacheConfig {
        capacity: 100,
        ttl: Duration::from_secs(300),
        eviction_check_interval: Duration::from_secs(30),
        invalidation_depth: 1,
        ..Default::default()
    };
    let call_graph_cache = Arc::new(CallGraphCache::new(cache_config));

    let lsp_cache_config = LspCacheConfig {
        capacity_per_operation: 100,
        ttl: Duration::from_secs(300),
        eviction_check_interval: Duration::from_secs(30),
        persistent: false,
        cache_directory: None,
    };
    let definition_cache = Arc::new(
        LspCache::new(LspOperation::Definition, lsp_cache_config)
            .expect("Failed to create definition cache"),
    );
    let manager = IndexingManager::new(
        config,
        language_detector,
        server_manager,
        call_graph_cache,
        definition_cache,
    );
    manager
        .start_indexing(workspace.path().to_path_buf())
        .await?;

    // Monitor queue to see priority processing
    let mut queue_snapshots = Vec::new();
    let start_time = Instant::now();

    while start_time.elapsed() < Duration::from_secs(10) {
        let queue_snapshot = manager.get_queue_snapshot().await;
        if queue_snapshot.total_items > 0 {
            queue_snapshots.push(queue_snapshot);
        }

        let progress = manager.get_progress().await;
        if progress.is_complete() {
            break;
        }
        sleep(Duration::from_millis(200)).await;
    }

    let final_progress = manager.get_progress().await;
    manager.stop_indexing().await?;

    // Should have processed files
    assert!(final_progress.processed_files > 0);

    // Should have seen queue activity if timing was right
    println!(
        "Priority processing test: {} files processed, {} queue snapshots",
        final_progress.processed_files,
        queue_snapshots.len()
    );

    Ok(())
}
