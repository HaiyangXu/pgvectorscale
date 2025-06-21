# pgvectorscale Technical Documentation

## Table of Contents
1. [Overview](#overview)
2. [PostgreSQL Extension Architecture](#postgresql-extension-architecture)
3. [Storage Layer Architecture](#storage-layer-architecture)
4. [DiskANN Algorithm Implementation](#diskann-algorithm-implementation)
5. [Index Building Process](#index-building-process)
6. [Query Processing](#query-processing)
7. [PostgreSQL Integration Details](#postgresql-integration-details)
8. [Performance Optimizations](#performance-optimizations)
9. [Memory Management](#memory-management)
10. [Configuration and Tuning](#configuration-and-tuning)

## Overview

pgvectorscale is a PostgreSQL extension that implements high-performance approximate nearest neighbor (ANN) search using the DiskANN algorithm. Built on the pgrx framework, it provides a custom access method that integrates seamlessly with PostgreSQL's query planner, storage system, and transaction management.

### Key Features
- **DiskANN Implementation**: Graph-based similarity search with optimized neighbor management
- **Dual Storage Systems**: Plain storage for accuracy vs SBQ (Scalar Binary Quantization) for memory efficiency
- **PostgreSQL Integration**: Full integration with buffer management, WAL logging, and query planning
- **Performance Optimization**: Advanced caching, quantization, and memory management strategies

## PostgreSQL Extension Architecture

### Extension Registration and Initialization

The extension is built using the pgrx framework and registers itself as a custom access method in PostgreSQL:

```rust
// Main extension entry point in lib.rs
pg_module_magic!();

#[pg_extern]
fn _PG_init() {
    // Extension initialization
    access_method::init();
}
```

### Access Method Interface

pgvectorscale implements PostgreSQL's `IndexAmRoutine` interface, providing handlers for all index operations:

- **`ambulkdelete`**: Bulk deletion during VACUUM operations
- **`amvacuumcleanup`**: Post-vacuum cleanup and statistics updates
- **`amcanreturn`**: Capability reporting for index-only scans
- **`amcostestimate`**: Query cost estimation for the planner
- **`amoptions`**: Index option parsing and validation
- **`amproperty`**: Index property queries
- **`amvalidate`**: Index validation and consistency checks
- **`ambuild`**: Index construction from heap data
- **`ambuildempty`**: Empty index creation
- **`aminsert`**: Single tuple insertion
- **`amrescan`**: Scan parameter updates
- **`amendscan`**: Scan cleanup
- **`amgettuple`**: Individual result retrieval
- **`amgetbitmap`**: Bitmap result generation
- **`ambeginscan`**: Scan initialization

### Custom Access Method Registration

The extension registers as `diskann` access method:

```sql
CREATE ACCESS METHOD diskann TYPE INDEX HANDLER diskann_handler;
```

This allows users to create indexes using:

```sql
CREATE INDEX ON table USING diskann (vector_column vector_l2_ops);
```

## Storage Layer Architecture

### Dual Storage Implementation

pgvectorscale implements two distinct storage strategies optimized for different use cases:

#### Plain Storage (`plain/storage.rs`)

**Purpose**: Maximum accuracy with full vector precision
**Use Case**: When accuracy is prioritized over memory usage

**Key Characteristics**:
- Stores full-precision vectors without quantization
- Direct distance calculations using original vector data
- Higher memory usage but maximum search accuracy
- Implements `Storage` trait with uncompressed data handling

**Data Structure**:
```rust
pub struct PlainStorage {
    // Full precision vector storage
    vectors: Vec<Vector>,
    // Direct neighbor relationships
    neighbors: NeighborStore,
}
```

#### SBQ Storage (`sbq/storage.rs`)

**Purpose**: Memory-efficient storage using Scalar Binary Quantization
**Use Case**: Large-scale deployments where memory efficiency is critical

**Key Characteristics**:
- Quantized vector storage using 1-bit per dimension
- Statistical preprocessing for optimal quantization
- Cached quantized vector access for performance
- Fallback to full vectors for final distance calculations

**Quantization Process**:
1. **Statistical Analysis**: Calculate mean and variance for each dimension across the dataset
2. **Bit Encoding**: Each dimension becomes 1 bit based on comparison to mean
3. **Distance Approximation**: Use XOR operations for fast similarity estimation
4. **Refinement**: Full vector calculations for final candidate ranking

### Page-Based Storage System

Both storage implementations use PostgreSQL's page-based storage system:

#### Page Structure (8KB pages)
- **Page Header**: PostgreSQL standard page header with LSN, checksum
- **Special Space**: Access method specific metadata
- **Item Pointers**: Array of pointers to items within the page
- **Free Space**: Available space for new items
- **Items**: Actual data items (vectors, neighbors, metadata)

#### Page Types
1. **Meta Page**: Index metadata and configuration
2. **Vector Pages**: Compressed or uncompressed vector data
3. **Neighbor Pages**: Graph connectivity information
4. **Quantization Pages**: Statistical data for SBQ compression

## DiskANN Algorithm Implementation

### Graph-Based Similarity Search

DiskANN implements approximate nearest neighbor search using a navigable small-world graph where each vector is a node connected to its most similar neighbors.

#### Core Algorithm Components

**1. Graph Construction (`graph/mod.rs`)**
- **Greedy Search**: Navigate from random start points toward query target
- **Neighbor Selection**: Choose diverse, high-quality neighbors using pruning algorithms
- **Graph Connectivity**: Maintain navigability while preventing clustering

**2. Neighbor Management (`graph/neighbor_store.rs`)**

Two distinct neighbor management systems:

**BuilderNeighborCache (In-Memory)**:
```rust
pub struct BuilderNeighborCache {
    neighbors: HashMap<u32, Vec<NeighborWithDistance>>,
    max_neighbors: usize,
}
```
- Used during index construction
- Allows dynamic neighbor list modification
- Supports efficient pruning and optimization

**DiskNeighborStore (Persistent)**:
```rust
pub struct DiskNeighborStore {
    page_manager: PageManager,
    neighbor_pages: BTreeMap<u32, PageId>,
}
```
- Used for production queries
- Immutable, disk-based storage
- Optimized for read performance

**3. Distance Calculations (`graph/neighbor_with_distance.rs`)**

Multi-tier distance calculation strategy:
1. **Quantized Distance**: Fast XOR-based approximation for SBQ vectors
2. **Full Distance**: Precise calculations for final ranking
3. **Tie Breaking**: Consistent ordering using node IDs

#### Graph Search Algorithm

**Greedy Search Process**:
1. **Initialization**: Start from multiple random entry points
2. **Expansion**: Explore neighbors of current best candidates
3. **Candidate Management**: Maintain priority queue of best candidates
4. **Termination**: Stop when no better neighbors are found
5. **Result Selection**: Return top-k candidates based on true distances

**Neighbor Pruning Strategy**:
- **Diversity**: Select neighbors that don't cluster in the same region
- **Connectivity**: Ensure graph remains navigable
- **Quality**: Prefer closer, high-quality connections
- **Robustness**: Maintain multiple paths to prevent disconnection

### Graph Construction Phases

#### Phase 1: Training and Statistics
```rust
fn build_phase_training() {
    // Collect representative sample for quantization
    // Calculate per-dimension statistics (mean, variance)
    // Initialize quantization parameters
}
```

#### Phase 2: Graph Building
```rust
fn build_phase_graph_construction() {
    // Insert vectors into growing graph
    // Perform greedy search for each new vector
    // Update neighbor relationships
    // Apply pruning algorithms
}
```

#### Phase 3: Optimization and Finalization
```rust
fn build_phase_finalization() {
    // Optimize graph connectivity
    // Finalize neighbor relationships
    // Write persistent graph structure
    // Generate index statistics
}
```

## Index Building Process

### Multi-Phase Construction

The index building process is divided into distinct phases for optimal resource utilization and progress tracking:

#### Phase Management
```rust
pub enum BuildPhase {
    Training,           // Statistical analysis and parameter tuning
    GraphConstruction,  // Core graph building
    Finalization,      // Optimization and persistence
}
```

### Heap Scanning and Vector Extraction

```rust
fn heap_scan_and_extract() {
    // Scan entire table using PostgreSQL's heap scanner
    // Extract vectors from tuples
    // Validate vector dimensions and format
    // Accumulate vectors for processing
}
```

#### Integration with PostgreSQL's Heap Scanner
- Uses `table_beginscan()` for efficient table traversal
- Processes tuples in buffer-friendly order
- Handles concurrent modifications through MVCC
- Respects transaction isolation levels

### Statistical Training Phase

For SBQ-enabled indexes, a training phase calculates optimal quantization parameters:

```rust
fn training_phase() {
    // Sample representative vectors from dataset
    // Calculate per-dimension mean and variance
    // Determine optimal quantization thresholds
    // Store quantization metadata in meta page
}
```

### Progress Reporting

The build process integrates with PostgreSQL's progress reporting system:

```rust
fn report_build_progress(phase: BuildPhase, progress: f64) {
    // Update pg_stat_progress_create_index view
    // Provide phase-specific progress metrics
    // Enable monitoring of long-running builds
}
```

## Query Processing

### Scan Initialization

Query processing begins with scan initialization in the access method:

```rust
fn ambeginscan() -> IndexScanDesc {
    // Parse query parameters (vector, k, search parameters)
    // Initialize scan state
    // Prepare distance calculation context
    // Set up result buffers
}
```

### Search Execution

#### Greedy Search Implementation
```rust
fn greedy_search(query_vector: &Vector, k: usize) -> Vec<Candidate> {
    let mut candidates = PriorityQueue::new();
    let mut visited = HashSet::new();
    
    // Start from random entry points
    for entry_point in select_entry_points() {
        candidates.push(entry_point);
    }
    
    while let Some(current) = candidates.pop() {
        if visited.contains(&current.node_id) {
            continue;
        }
        visited.insert(current.node_id);
        
        // Explore neighbors
        for neighbor in get_neighbors(current.node_id) {
            let distance = calculate_distance(query_vector, neighbor.vector);
            candidates.push(Candidate::new(neighbor.node_id, distance));
        }
        
        // Terminate if no improvement
        if candidates.len() >= k && current.distance > candidates.peek().distance {
            break;
        }
    }
    
    candidates.into_sorted_vec()
}
```

#### Distance Calculation Optimization

Multi-stage distance calculation for optimal performance:

1. **Quick Filtering**: Use quantized distances for initial filtering
2. **Candidate Refinement**: Full distance calculations for promising candidates
3. **Final Ranking**: Precise ordering of top-k results

### Result Management

```rust
fn amgetttuple() -> bool {
    // Return next result from current scan
    // Handle end-of-scan conditions
    // Manage scan state transitions
}

fn amgetbitmap() -> TIDBitmap {
    // Generate bitmap for bitmap index scans
    // Optimize for large result sets
    // Integrate with PostgreSQL's bitmap heap scan
}
```

## PostgreSQL Integration Details

### Buffer Management Integration

pgvectorscale integrates tightly with PostgreSQL's buffer pool for optimal memory management:

#### Buffer Operations (`util/buffer.rs`)
```rust
fn get_buffer(relation: Relation, block_num: BlockNumber) -> Buffer {
    // Acquire buffer from PostgreSQL's shared buffer pool
    // Handle buffer locking and concurrency
    // Manage buffer replacement policies
}

fn release_buffer(buffer: Buffer) {
    // Release buffer back to pool
    // Update access statistics
    // Trigger write-back if dirty
}
```

#### Page Management (`util/page.rs`)
```rust
fn init_page(page: Page, page_type: PageType) {
    // Initialize PostgreSQL page header
    // Set up special space for access method data
    // Configure page for specific data type
}

fn get_item_pointer(page: Page, offset: OffsetNumber) -> ItemPointer {
    // Access item within page using offset
    // Handle item boundaries and validation
    // Return pointer to actual data
}
```

### Write-Ahead Logging (WAL) Integration

All modifications are logged through PostgreSQL's WAL system for crash recovery:

```rust
fn log_index_operation(operation: IndexOperation) {
    // Generate WAL record for operation
    // Include necessary redo/undo information
    // Ensure consistency across crashes
}
```

#### WAL Record Types
- **Index Creation**: Complete index structure
- **Vector Insertion**: New vector and graph updates
- **Neighbor Updates**: Graph connectivity changes
- **Quantization Updates**: Statistical parameter changes

### Transaction Integration

The access method respects PostgreSQL's ACID properties:

- **Atomicity**: All index changes within a transaction are atomic
- **Consistency**: Index maintains graph connectivity invariants
- **Isolation**: MVCC integration for concurrent access
- **Durability**: WAL logging ensures persistence

### Query Planner Integration

#### Cost Estimation (`access_method/cost_estimate.rs`)

The cost estimator provides the query planner with accurate estimates for different access patterns:

```rust
fn cost_estimate(
    path: &IndexPath,
    index_pages: f64,
    index_tuples: f64,
    search_parameters: &SearchParams
) -> Cost {
    // Estimate I/O costs based on index size and access pattern
    // Calculate CPU costs for distance computations
    // Factor in selectivity and result set size
    // Consider cache effects and sequential vs random access
}
```

#### Statistics Collection (`access_method/stats.rs`)

Runtime statistics help the planner make optimal decisions:

```rust
pub struct IndexStatistics {
    pub total_vectors: u32,
    pub avg_neighbors_per_node: f32,
    pub quantization_effectiveness: f32,
    pub search_path_length_avg: f32,
}
```

## Performance Optimizations

### Quantization System

#### Scalar Binary Quantization (SBQ)

The SBQ system reduces memory usage by ~32x while maintaining search quality:

**Statistical Preprocessing**:
```rust
fn calculate_quantization_stats(vectors: &[Vector]) -> QuantizationStats {
    let n_dims = vectors[0].len();
    let mut means = vec![0.0; n_dims];
    let mut variances = vec![0.0; n_dims];
    
    // Calculate per-dimension means
    for vector in vectors {
        for (i, &value) in vector.iter().enumerate() {
            means[i] += value;
        }
    }
    for mean in &mut means {
        *mean /= vectors.len() as f32;
    }
    
    // Calculate per-dimension variances
    for vector in vectors {
        for (i, &value) in vector.iter().enumerate() {
            let diff = value - means[i];
            variances[i] += diff * diff;
        }
    }
    for variance in &mut variances {
        *variance /= vectors.len() as f32;
    }
    
    QuantizationStats { means, variances }
}
```

**Bit Encoding**:
```rust
fn quantize_vector(vector: &Vector, stats: &QuantizationStats) -> QuantizedVector {
    let mut bits = BitVec::new();
    for (i, &value) in vector.iter().enumerate() {
        bits.push(value > stats.means[i]);
    }
    QuantizedVector { bits }
}
```

**Fast Distance Calculation**:
```rust
fn quantized_distance(a: &QuantizedVector, b: &QuantizedVector) -> u32 {
    // XOR for bit difference, popcount for Hamming distance
    (a.bits.as_raw_slice() ^ b.bits.as_raw_slice())
        .iter()
        .map(|&x| x.count_ones())
        .sum()
}
```

### Caching Strategies

#### Vector Caching (`sbq/cache.rs`)

Multi-level caching system for optimal access patterns:

```rust
pub struct QuantizedVectorCache {
    // L1: Recently accessed quantized vectors
    quantized_cache: LruCache<NodeId, QuantizedVector>,
    
    // L2: Full vectors for distance refinement
    full_vector_cache: LruCache<NodeId, Vector>,
    
    // L3: Preloaded neighbor sets
    neighbor_cache: LruCache<NodeId, Vec<NodeId>>,
}
```

#### Cache Management
- **LRU Eviction**: Least recently used vectors are evicted first
- **Preloading**: Anticipatory loading of likely-needed neighbors
- **Size Limits**: Configurable cache sizes based on available memory
- **Hit Rate Optimization**: Dynamic adjustment based on access patterns

### Graph Optimization

#### Neighbor Pruning Algorithms

Advanced pruning maintains graph quality while controlling memory usage:

```rust
fn prune_neighbors(
    candidates: Vec<NeighborWithDistance>,
    max_neighbors: usize,
    alpha: f32  // Diversity parameter
) -> Vec<NeighborWithDistance> {
    let mut selected = Vec::new();
    let mut remaining = candidates;
    remaining.sort_by_key(|n| n.distance);
    
    while selected.len() < max_neighbors && !remaining.is_empty() {
        let next = remaining.remove(0);
        
        // Check diversity constraint
        let mut accept = true;
        for existing in &selected {
            let angle = calculate_angle(next.vector, existing.vector);
            if angle < alpha {
                accept = false;
                break;
            }
        }
        
        if accept {
            selected.push(next);
        }
    }
    
    selected
}
```

#### Graph Connectivity Maintenance

Algorithms ensure the graph remains navigable:
- **Weak Component Detection**: Identify disconnected regions
- **Bridge Insertion**: Add connections to maintain navigability
- **Degree Balance**: Distribute connections evenly across the graph

## Memory Management

### Deadlock Prevention

Careful buffer management prevents deadlocks in concurrent scenarios:

```rust
fn acquire_buffers_ordered(blocks: &mut [BlockNumber]) {
    // Sort blocks to ensure consistent acquisition order
    blocks.sort();
    
    for &block in blocks {
        // Acquire buffers in sorted order to prevent deadlocks
        acquire_buffer_with_lock(block);
    }
}
```

### Memory Pool Management

Custom memory pools for high-frequency allocations:

```rust
pub struct VectorPool {
    // Pool of reusable vector buffers
    free_vectors: Vec<Vector>,
    // Pool of distance calculation workspaces
    distance_workspaces: Vec<DistanceWorkspace>,
}
```

### Batch Processing

Operations are batched to minimize overhead:

```rust
fn batch_vector_insertions(vectors: Vec<(TupleId, Vector)>) {
    // Group insertions by target page
    let mut page_groups = HashMap::new();
    for (tid, vector) in vectors {
        let page_id = calculate_target_page(vector);
        page_groups.entry(page_id).or_insert(Vec::new()).push((tid, vector));
    }
    
    // Process each page group atomically
    for (page_id, page_vectors) in page_groups {
        process_page_insertions(page_id, page_vectors);
    }
}
```

## Configuration and Tuning

### Index Options (`access_method/options.rs`)

Comprehensive configuration system for optimal performance:

```rust
pub struct DiskAnnOptions {
    // Algorithm parameters
    pub num_neighbors: u32,          // Neighbors per node in graph
    pub search_list_size: u32,       // Search beam width
    pub max_alpha: f32,              // Diversity parameter for pruning
    
    // Storage configuration
    pub storage_layout: StorageLayout, // Plain vs SBQ
    pub quantization_ratio: f32,      // Quantization aggressiveness
    
    // Performance tuning
    pub cache_size: usize,           // Cache size in MB
    pub batch_size: u32,             // Batch processing size
    pub parallel_build: bool,        // Enable parallel construction
    
    // Memory management
    pub work_mem_limit: usize,       // Memory limit for operations
    pub maintenance_work_mem: usize, // Memory for maintenance operations
}
```

### GUC (Grand Unified Configuration) Integration (`access_method/guc.rs`)

PostgreSQL configuration integration:

```sql
-- Algorithm tuning
SET diskann.num_neighbors = 50;
SET diskann.search_list_size = 100;
SET diskann.max_alpha = 1.2;

-- Memory management
SET diskann.cache_size = '512MB';
SET diskann.work_mem_limit = '1GB';

-- Performance optimization
SET diskann.enable_parallel_build = true;
SET diskann.batch_size = 1000;
```

### Adaptive Configuration

Runtime adaptation based on workload characteristics:

```rust
fn adapt_configuration(stats: &WorkloadStats) -> DiskAnnOptions {
    let mut options = get_default_options();
    
    // Adapt to dataset size
    if stats.total_vectors > 1_000_000 {
        options.storage_layout = StorageLayout::SBQ;
        options.quantization_ratio = 0.8;
    }
    
    // Adapt to query patterns
    if stats.avg_k > 100 {
        options.search_list_size *= 2;
    }
    
    // Adapt to memory constraints
    if stats.available_memory < (1 << 30) { // < 1GB
        options.cache_size = stats.available_memory / 4;
    }
    
    options
}
```

## Debugging and Monitoring

### Debug Information (`access_method/debugging.rs`)

Comprehensive debugging support for development and troubleshooting:

```rust
pub fn dump_index_structure(index: &DiskAnnIndex) {
    // Dump graph connectivity statistics
    info!("Graph nodes: {}", index.node_count());
    info!("Average degree: {:.2}", index.average_degree());
    info!("Connected components: {}", index.connected_components());
    
    // Dump storage statistics
    info!("Storage layout: {:?}", index.storage_layout());
    info!("Quantization ratio: {:.2}", index.quantization_ratio());
    info!("Memory usage: {} MB", index.memory_usage() / (1 << 20));
}
```

### Performance Monitoring

Integration with PostgreSQL's statistics system:

```sql
-- Query index statistics
SELECT * FROM pg_stat_user_indexes WHERE indexrelname = 'my_diskann_idx';

-- View build progress
SELECT * FROM pg_stat_progress_create_index;

-- Monitor cache effectiveness
SELECT 
    cache_hits,
    cache_misses,
    hit_ratio
FROM pg_diskann_stats;
```

This comprehensive technical documentation covers the complete architecture and implementation of pgvectorscale, from its PostgreSQL integration through its DiskANN algorithm implementation to its performance optimization strategies. The documentation serves as both a reference for understanding the codebase and a guide for optimization and troubleshooting.
