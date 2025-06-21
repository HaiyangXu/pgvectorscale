# Database Storage Systems: Page-Based vs Direct Disk Access

## Introduction

This document provides a comprehensive guide to understanding database storage systems, specifically focusing on how databases manage data on disk versus in memory. If you're a software engineer working with databases, particularly PostgreSQL extensions like **pgvectorscale**, understanding these concepts is crucial for building efficient data storage and retrieval systems.

**Why This Matters:**
- Learn how databases achieve high performance through smart storage strategies
- Understand the trade-offs between different storage approaches
- Gain insight into why PostgreSQL uses page-based storage for the pgvectorscale vector extension
- Discover how different database types (relational, NoSQL, graph, etc.) solve storage challenges

**Who This Is For:**
- Software engineers new to database internals
- Developers working with PostgreSQL extensions
- Anyone curious about how databases store and retrieve data efficiently
- Developers building data-intensive applications

**What You'll Learn:**
1. The fundamental difference between direct file access and page-based storage
2. How PostgreSQL's 8KB page system works under the hood
3. Why page-based storage enables transactions, caching, and concurrency
4. How different database types (MongoDB, Redis, Cassandra, etc.) approach storage
5. When to choose different storage strategies for your applications

## Overview

This document explains the differences between page-based storage (used by most relational databases) and direct disk access, written for software engineers new to database internals.

## Direct Disk Access (What You Might Expect)

As a software engineer, you might think databases work like this:

```c
// Naive approach - direct file operations
FILE* data_file = fopen("my_vectors.dat", "rb");

// Read a specific vector directly
fseek(data_file, vector_id * sizeof(Vector), SEEK_SET);
Vector my_vector;
fread(&my_vector, sizeof(Vector), 1, data_file);

// Write a new vector
fseek(data_file, 0, SEEK_END);
fwrite(&new_vector, sizeof(Vector), 1, data_file);
fclose(data_file);
```

### Problems with Direct Disk Access

1. **Expensive I/O**: Every read/write hits the disk
2. **No Caching**: No memory optimization
3. **No Concurrency**: File locking issues with multiple users
4. **No Transactions**: No way to rollback changes
5. **No Structure**: Hard to manage complex data relationships

## Page-Based Storage (Database Approach)

Databases use **page-based storage** - think of it as "smart chunking" of data.

### What is a Page?

A **page** is a fixed-size block of data (usually 4KB, 8KB, or 16KB). PostgreSQL uses **8KB pages**.

```rust
// PostgreSQL page structure (simplified)
struct Page {
    header: PageHeader,     // 24 bytes - metadata
    item_pointers: Vec<ItemPointer>, // Directory of items on page
    free_space: [u8],      // Unused space
    actual_data: [u8],     // Your vectors, rows, etc.
    special_data: [u8],    // Index-specific data
}

// Page header contains:
struct PageHeader {
    page_size: u16,        // Always 8192 for PostgreSQL
    version: u16,          // Page format version
    lower: u16,            // Offset to start of free space
    upper: u16,            // Offset to end of free space
    special: u16,          // Offset to special data
    page_size_and_version: u16,
    transaction_info: TransactionId,
    checksum: u16,         // Data integrity check
}
```

### How Page-Based Storage Works

#### 1. Chunked I/O Operations

Instead of reading individual records, you read entire pages:

```rust
// Instead of reading one vector at a time
fn read_vector_direct(vector_id: u32) -> Vector {
    // Bad: One disk I/O per vector
    disk_seek(vector_id * VECTOR_SIZE);
    disk_read(VECTOR_SIZE)
}

// Page-based approach
fn read_vector_paged(vector_id: u32) -> Vector {
    let page_id = vector_id / VECTORS_PER_PAGE;
    let offset_in_page = vector_id % VECTORS_PER_PAGE;
    
    // Good: One disk I/O gets multiple vectors
    let page = read_page(page_id);  // Reads 8KB containing ~100 vectors
    return page.get_vector(offset_in_page);
}
```

#### 2. Buffer Pool Management

Pages are cached in memory (the "buffer pool"):

```rust
// Simplified buffer pool
struct BufferPool {
    pages: HashMap<PageId, Page>,  // In-memory cache
    dirty_pages: HashSet<PageId>,  // Pages modified but not written to disk
    lru_list: LinkedList<PageId>,  // Least Recently Used for eviction
}

impl BufferPool {
    fn get_page(&mut self, page_id: PageId) -> &mut Page {
        if let Some(page) = self.pages.get_mut(&page_id) {
            // Cache hit - no disk I/O needed!
            self.lru_list.move_to_front(page_id);
            return page;
        }
        
        // Cache miss - need to load from disk
        if self.pages.len() >= MAX_BUFFER_SIZE {
            self.evict_lru_page();  // Make room
        }
        
        let page = self.load_page_from_disk(page_id);  // Expensive disk I/O
        self.pages.insert(page_id, page);
        self.pages.get_mut(&page_id).unwrap()
    }
}
```

### Real-World Example: Storing Vectors

Let's say you're storing 1000-dimensional float vectors (4KB each):

#### Direct Disk Approach:
```rust
// File: vectors.dat
// Vector 0: bytes 0-4095
// Vector 1: bytes 4096-8191  
// Vector 2: bytes 8192-12287
// ...

// To read vector 100:
fseek(file, 100 * 4096, SEEK_SET);  // Seek to byte 409,600
fread(buffer, 4096, 1, file);       // Read 4KB
// Result: 1 disk seek + 1 disk read per vector
```

#### Page-Based Approach:
```rust
// Page 0: Contains vectors 0-1    (8KB page holds 2 vectors)
// Page 1: Contains vectors 2-3
// Page 50: Contains vectors 100-101
// ...

// To read vector 100:
let page_id = 100 / 2;              // Page 50
let page = buffer_pool.get_page(50); // May be cached!
let vector = page.get_vector(100 % 2); // Extract vector from page
// Result: Often 0 disk I/O (cache hit), max 1 disk read gets 2 vectors
```

## Key Benefits of Page-Based Storage

### 1. Spatial Locality

Related data is stored together:

```rust
// In pgvectorscale plain storage
struct VectorPage {
    vectors: [Vector; 32],           // 32 vectors per 8KB page
    graph_neighbors: [Vec<u32>; 32], // DiskANN graph links
    metadata: PageMetadata,
}

// When you search for similar vectors, you often need their neighbors too
// Page-based storage loads them all in one I/O operation
```

### 2. Efficient Caching

```rust
// Example: Searching for nearest neighbors
fn find_nearest_neighbors(query: &Vector, k: usize) -> Vec<Vector> {
    let mut candidates = Vec::new();
    
    // Start at entry point
    let entry_page = buffer_pool.get_page(ENTRY_PAGE_ID); // Likely cached
    
    // Traverse graph - many pages will be cache hits
    for neighbor_page_id in entry_page.get_neighbor_pages() {
        let page = buffer_pool.get_page(neighbor_page_id); // Often cached!
        candidates.extend(page.get_all_vectors());
    }
    
    // Most page reads are served from memory, not disk
    candidates.sort_by_distance(query).take(k)
}
```

### 3. Transactional Support

```rust
// Pages enable atomic operations
fn insert_vector_with_transaction(vector: Vector) -> Result<()> {
    let mut transaction = begin_transaction();
    
    // 1. Lock the target page
    let page = buffer_pool.get_page_exclusive(target_page_id);
    
    // 2. Make changes in memory
    page.insert_vector(vector);
    buffer_pool.mark_dirty(target_page_id);
    
    // 3. Log changes for crash recovery
    write_ahead_log.log_page_change(target_page_id, &page);
    
    // 4. Commit - all changes written atomically
    transaction.commit(); // Flushes dirty pages to disk
    
    Ok(())
}
```

### 4. Concurrency Control

```rust
// Multiple readers/writers can work on different pages
struct PageLocks {
    page_locks: HashMap<PageId, RwLock<()>>,
}

// Reader 1: Searching vectors 0-31 (Page 0)
let page_0 = buffer_pool.get_page(0);
let _read_lock = page_locks.read_lock(0); // Shared lock

// Reader 2: Searching vectors 64-95 (Page 2) - can run concurrently!
let page_2 = buffer_pool.get_page(2);
let _read_lock = page_locks.read_lock(2); // Different page, no conflict

// Writer: Inserting into page 1 - doesn't block the readers
let page_1 = buffer_pool.get_page_mut(1);
let _write_lock = page_locks.write_lock(1); // Exclusive lock on page 1 only
```

## Performance Comparison

### Direct File Access:
- **Seeks**: High - one per operation
- **Cache**: None - always hits disk  
- **Concurrency**: Poor - file-level locking
- **Consistency**: Manual - no transaction support

### Page-Based Storage:
- **Seeks**: Low - amortized across multiple records
- **Cache**: Excellent - frequently accessed pages stay in memory
- **Concurrency**: Good - page-level locking
- **Consistency**: Built-in - ACID transactions

## Real Numbers Example

Imagine querying 1000 vectors for nearest neighbor search:

**Direct Disk Access:**
- 1000 disk seeks × 10ms = 10 seconds
- 1000 disk reads × 1ms = 1 second  
- **Total: 11 seconds**

**Page-Based with 90% cache hit rate:**
- 100 cache misses × 11ms (seek + read) = 1.1 seconds
- 900 cache hits × 0.001ms = 0.9ms
- **Total: ~1.1 seconds**

That's a **10x performance improvement** just from caching!

---

# Database Storage Types: Which Databases Use What?

**Not all databases use page-based storage**, but most traditional relational databases do. Here's a breakdown by database types:

## Databases That Use Page-Based Storage

### Relational Databases (Most Common)
- **PostgreSQL**: 8KB pages
- **MySQL/InnoDB**: 16KB pages (configurable)
- **SQL Server**: 8KB pages
- **Oracle**: 8KB pages (default, configurable)
- **SQLite**: Variable page sizes (512B to 64KB)
- **DB2**: 4KB, 8KB, 16KB, or 32KB pages

```sql
-- You can even see page info in PostgreSQL
SELECT 
    schemaname, tablename, 
    pg_size_pretty(pg_total_relation_size(schemaname||'.'||tablename)) as size,
    pg_relation_size(schemaname||'.'||tablename) / 8192 as pages
FROM pg_tables 
WHERE schemaname = 'public';
```

## Databases That DON'T Use Traditional Page Storage

### 1. Log-Structured Databases

**Examples**: Apache Cassandra, HBase, LevelDB, RocksDB

Instead of pages, they use **log-structured merge trees (LSM)**:

```rust
// LSM approach - write everything as logs
struct LSMStorage {
    memtable: BTreeMap<Key, Value>,     // In-memory writes
    sstables: Vec<SortedStringTable>,   // Immutable sorted files on disk
}

// Writes go to memory first, then get flushed as sorted files
fn write(key: Key, value: Value) {
    memtable.insert(key, value);
    if memtable.size() > THRESHOLD {
        flush_to_sstable();  // Write entire memtable as one file
    }
}
```

**Why no pages?**
- Optimized for **write-heavy workloads**
- Avoids random writes (everything is sequential)
- Better for distributed systems

### 2. Column-Store Databases

**Examples**: Apache Parquet, ClickHouse, Amazon Redshift, Google BigQuery

Store data by **columns instead of rows**:

```rust
// Traditional row-based (page storage)
// Page contains: [Row1: id,name,age | Row2: id,name,age | Row3: id,name,age]

// Column-based storage
struct ColumnStore {
    id_column: Vec<i32>,      // [1, 2, 3, 4, 5, ...]
    name_column: Vec<String>, // ["Alice", "Bob", "Charlie", ...]  
    age_column: Vec<i32>,     // [25, 30, 35, 40, 45, ...]
}
```

**Why no traditional pages?**
- Better compression (similar values grouped together)
- Faster analytics queries (only read needed columns)
- Vectorized processing

### 3. In-Memory Databases

**Examples**: Redis, Memcached, SAP HANA, VoltDB

Store everything in RAM:

```rust
// Redis-style storage
struct InMemoryDB {
    data: HashMap<String, Value>,  // Everything in memory
    persistence: Option<AOFLog>,   // Optional disk backup
}

// No pages needed - direct memory access
fn get(key: &str) -> Option<Value> {
    data.get(key).cloned()  // Direct HashMap lookup
}
```

**Why no pages?**
- RAM is fast enough for direct access
- No need to manage disk I/O
- Pages add unnecessary overhead

### 4. Object Databases

**Examples**: MongoDB (newer versions), Amazon S3, Azure Blob Storage

Store **documents/objects** instead of structured data:

```rust
// Document storage (MongoDB-style)
struct DocumentStore {
    collections: HashMap<String, Vec<Document>>,
}

struct Document {
    id: ObjectId,
    data: BSONDocument,  // Variable-size document
    indexes: Vec<IndexEntry>,
}
```

**Storage approach:**
- **WiredTiger** (MongoDB's engine): Uses **B-tree pages** (similar to traditional)
- **GridFS**: Splits large files into **chunks** (not fixed-size pages)

### 5. Graph Databases

**Examples**: Neo4j, Amazon Neptune, ArangoDB

Optimized for **relationships between data**:

```rust
// Graph storage focuses on relationships
struct GraphStorage {
    nodes: HashMap<NodeId, Node>,
    edges: HashMap<EdgeId, Edge>,
    adjacency_lists: HashMap<NodeId, Vec<NodeId>>,
}

// Different storage patterns for different query types
// - Adjacency lists for traversal
// - Property stores for node/edge data
// - Index structures for fast lookups
```

## Hybrid Approaches

### Modern Databases Often Mix Approaches

**PostgreSQL** (which pgvectorscale extends):
- **Regular tables**: 8KB pages
- **TOAST**: Large objects stored separately
- **Extensions**: Can implement custom storage (like pgvectorscale)

```sql
-- PostgreSQL can have different storage for different data types
CREATE TABLE mixed_storage (
    id SERIAL,                    -- Regular page storage
    description TEXT,             -- TOAST storage if large
    embedding vector(1536)        -- pgvectorscale custom storage
);
```

**MySQL**:
- **InnoDB**: 16KB pages
- **MyISAM**: Different file-based approach  
- **Memory Engine**: In-memory storage

## Why Page Storage Remains Popular

Despite alternatives, page-based storage is still dominant because:

### 1. ACID Transactions

```rust
// Pages make transactions easier
fn transfer_money(from: Account, to: Account, amount: Money) {
    let mut txn = begin_transaction();
    
    // Both accounts might be on same page - atomic update
    let page = get_page_containing_accounts(from, to);
    page.debit(from, amount);
    page.credit(to, amount);
    
    txn.commit(); // Single page write = atomic operation
}
```

### 2. Mature Ecosystem
- **Query optimizers** understand page costs
- **Backup/recovery** tools work with pages
- **Replication** systems stream page changes

### 3. Balanced Performance
- Good for both **OLTP** (many small transactions) and **OLAP** (analytical queries)
- Reasonable performance across different workload patterns

## Summary Table

| Database Type | Storage Model | Use Case |
|---------------|---------------|----------|
| **PostgreSQL, MySQL** | Fixed-size pages | General purpose OLTP |
| **Cassandra, HBase** | Log-structured | Write-heavy, distributed |
| **ClickHouse, BigQuery** | Columnar | Analytics, data warehousing |
| **Redis, Memcached** | In-memory | Caching, session storage |
| **Neo4j, Neptune** | Graph-optimized | Relationship-heavy data |
| **MongoDB** | Document + pages | Semi-structured data |

## Conclusion

The choice depends on your **workload characteristics**:
- **High write volume** → LSM trees
- **Analytics queries** → Columnar storage  
- **Complex relationships** → Graph databases
- **General purpose** → Page-based storage

Page-based storage remains the "Swiss Army knife" of database storage - not always optimal, but good enough for most use cases.

---

*This document was created to explain database storage systems in the context of the pgvectorscale PostgreSQL extension, which implements custom vector storage using PostgreSQL's page-based infrastructure.*
