# pgvectorscale Query Execution Workflow

## Overview

This document provides a detailed explanation of the query planning and execution workflow in pgvectorscale, covering how queries are processed from PostgreSQL's query planner through the DiskANN index access methods to return results.

## Table of Contents

1. [Query Execution Flow Overview](#query-execution-flow-overview)
2. [PostgreSQL Integration Points](#postgresql-integration-points)
3. [Access Method Interface Implementation](#access-method-interface-implementation)
4. [Query Processing Phases](#query-processing-phases)
5. [Label Filtering vs WHERE Clause Filtering](#label-filtering-vs-where-clause-filtering)
6. [Storage-Specific Query Execution](#storage-specific-query-execution)
7. [Graph Search Algorithm](#graph-search-algorithm)
8. [Performance Optimizations](#performance-optimizations)

## Query Execution Flow Overview

The complete query execution flow follows this path:

```
PostgreSQL Query Planner
    ↓
Cost Estimation (amcostestimate)
    ↓
Scan Initialization (ambeginscan)
    ↓
Query Setup (amrescan)
    ↓
Result Iteration (amgettuple)
    ↓
Scan Cleanup (amendscan)
```

## PostgreSQL Integration Points

### Access Method Registration

pgvectorscale registers as a custom access method in PostgreSQL:

```rust
// File: src/access_method/mod.rs
CREATE ACCESS METHOD diskann TYPE INDEX HANDLER diskann_amhandler;
```

The access method handler provides implementations for all required PostgreSQL index operations:

```rust
amroutine.amvalidate = Some(amvalidate);
amroutine.ambuild = Some(build::ambuild);
amroutine.ambuildempty = Some(build::ambuildempty);
amroutine.aminsert = Some(build::aminsert);
amroutine.ambulkdelete = Some(vacuum::ambulkdelete);
amroutine.amvacuumcleanup = Some(vacuum::amvacuumcleanup);
amroutine.amcostestimate = Some(cost_estimate::amcostestimate);
amroutine.amoptions = Some(options::amoptions);
amroutine.ambeginscan = Some(scan::ambeginscan);
amroutine.amrescan = Some(scan::amrescan);
amroutine.amgettuple = Some(scan::amgettuple);
amroutine.amendscan = Some(scan::amendscan);
```

## Access Method Interface Implementation

### 1. Scan Initialization (`ambeginscan`)

**Purpose**: Initialize a new index scan
**File**: `src/access_method/scan.rs`

```rust
#[pg_guard]
pub extern "C" fn ambeginscan(
    index_relation: pg_sys::Relation,
    nkeys: ::std::os::raw::c_int,
    norderbys: ::std::os::raw::c_int,
) -> pg_sys::IndexScanDesc
```

**Key Operations**:
- Creates a `TSVScanState` structure to maintain scan context
- Fetches the index meta page containing configuration
- Registers PostgreSQL statistics for query tracking
- Allocates memory context for the scan state

### 2. Query Setup (`amrescan`)

**Purpose**: Set up or reset a scan with specific query parameters
**File**: `src/access_method/scan.rs`

```rust
#[pg_guard]
pub extern "C" fn amrescan(
    scan: pg_sys::IndexScanDesc,
    keys: pg_sys::ScanKey,
    nkeys: ::std::os::raw::c_int,
    orderbys: pg_sys::ScanKey,
    norderbys: ::std::os::raw::c_int,
)
```

**Key Operations**:
- Validates input parameters (expects 1 ORDER BY key, 0-1 WHERE keys)
- Extracts query vector and label filters from scan keys
- Determines search list size from GUC parameters
- Initializes storage-specific scan iterators
- Sets up the appropriate distance function

**Query Vector Extraction**:
```rust
let query = unsafe { LabeledVector::from_scan_key_data(keys, orderby_keys, &state.meta_page) };
```

### 3. Result Iteration (`amgettuple`)

**Purpose**: Return the next tuple from the index scan
**File**: `src/access_method/scan.rs`

```rust
#[pg_guard]
pub extern "C" fn amgettuple(
    scan: pg_sys::IndexScanDesc,
    _direction: pg_sys::ScanDirection::Type,
) -> bool
```

**Key Operations**:
- Delegates to storage-specific iterator (`TSVResponseIterator`)
- Handles both Plain and SBQ storage implementations
- Manages deleted tuple detection and skipping
- Maintains index page pins for PostgreSQL locking protocol
- Returns heap tuple ID for result fetching

### 4. Scan Cleanup (`amendscan`)

**Purpose**: Clean up scan resources and log statistics
**File**: `src/access_method/scan.rs`

```rust
#[pg_guard]
pub extern "C" fn amendscan(scan: pg_sys::IndexScanDesc)
```

**Key Operations**:
- Logs detailed query statistics at DEBUG1 level
- Validates quantizer statistics consistency
- Releases scan state memory

## Query Processing Phases

### Phase 1: Query Planning and Cost Estimation

PostgreSQL's query planner calls `amcostestimate` to determine whether to use the index:

```rust
// File: src/access_method/cost_estimate.rs
#[pg_guard]
pub extern "C" fn amcostestimate(/* parameters */) -> bool
```

**Cost Factors Considered**:
- Index pages that need to be accessed
- Random vs sequential I/O patterns
- Comparison with sequential scan costs
- Label filtering selectivity estimates

### Phase 2: Scan State Initialization

The `TSVScanState` structure maintains the complete scan context:

```rust
struct TSVScanState {
    storage: *mut StorageState,           // Storage-specific iterator
    distance_fn: Option<DistanceFn>,      // Distance calculation function
    meta_page: MetaPage,                  // Index metadata
    last_buffer: Option<PinnedBufferShare>, // Last accessed index page
}
```

**Storage State Types**:
```rust
enum StorageState {
    SbqSpeedup(
        SbqQuantizer,
        TSVResponseIterator<SbqSearchDistanceMeasure, SbqSpeedupStorageLsnPrivateData>,
    ),
    Plain(TSVResponseIterator<PlainDistanceMeasure, PlainStorageLsnPrivateData>),
}
```

### Phase 3: Graph Search Initialization

The query execution initializes a streaming graph search:

```rust
// File: src/access_method/graph/mod.rs
pub fn greedy_search_streaming_init<S: Storage>(
    &self,
    query: LabeledVector,
    search_list_size: usize,
    storage: &S,
) -> ListSearchResult<S::QueryDistanceMeasure, S::LSNPrivateData>
```

**Key Components**:
- **Start Nodes**: Entry points into the graph, chosen based on label filters
- **Search List Size**: Controls the beam width for the search algorithm
- **Distance Measure**: Storage-specific distance calculation (quantized vs full precision)

### Phase 4: Iterative Result Generation

Each call to `amgettuple` advances the graph search:

```rust
// Core iteration logic in TSVResponseIterator
fn next<S: Storage<QueryDistanceMeasure = QDM, LSNPrivateData = PD>>(
    &mut self,
    storage: &S,
) -> Option<(HeapPointer, IndexPointer)>
```

**Iteration Steps**:
1. **Graph Traversal**: Visit neighbors of the closest unvisited nodes
2. **Label Filtering**: Skip nodes that don't match label criteria (if applicable)
3. **Distance Calculation**: Compute distances to candidate neighbors
4. **Priority Queue Management**: Maintain candidates sorted by distance
5. **Deleted Tuple Handling**: Skip tuples marked as deleted
6. **Result Return**: Return heap pointer for tuple fetching

## Label Filtering vs WHERE Clause Filtering

### Label Filtering (Optimized Path)

**Implementation**: Direct integration during graph traversal
**Query Pattern**: `WHERE labels && ARRAY[1,2,3]`
**Performance**: High - filtering happens during graph search

**Key Implementation Details**:
- Labels are stored as `smallint[]` arrays in the index
- Filtering uses the PostgreSQL `&&` (overlap) operator
- Label checks occur during neighbor visitation:

```rust
// File: src/access_method/sbq/storage.rs
if let Some(labels) = lsr.sdm.as_ref().expect("sdm is Some").query.labels() {
    if !no_filter
        && !node_neighbor
            .get_labels()
            .as_ref()
            .map_or(false, |node_labels| labels.overlaps(node_labels))
    {
        // Skip nodes without matching labels
        continue;
    }
}
```

**Query Flow with Label Filtering**:
1. Extract label filter from `WHERE labels && ARRAY[...]` clause
2. Choose label-appropriate start nodes
3. During graph traversal, skip nodes without matching labels
4. Return only results that pass label filtering

### WHERE Clause Filtering (Post-Processing Path)

**Implementation**: Applied after vector search results are retrieved
**Query Pattern**: `WHERE status = 'active' AND created_at > '2024-01-01'`
**Performance**: Lower - requires checking all vector results against filter

**Query Flow with WHERE Clause Filtering**:
1. Perform full vector search ignoring arbitrary WHERE conditions
2. PostgreSQL applies WHERE conditions to results post-scan
3. May require scanning more results than requested LIMIT

## Storage-Specific Query Execution

### Plain Storage Query Execution

**Characteristics**:
- Full precision distance calculations
- Direct vector comparisons
- Higher accuracy, higher memory usage

**Query Path**:
```rust
StorageState::Plain(iter) => {
    let storage = PlainStorage::load_for_search(&indexrel, &heaprel, state.distance_fn.unwrap());
    let next = if state.meta_page.get_num_dimensions() == state.meta_page.get_num_dimensions_to_index() {
        iter.next(&storage)  // No resorting needed
    } else {
        iter.next_with_resort(&scan, &indexrel, &storage)  // Resort with full precision
    };
    get_tuple(state, next, scan)
}
```

### SBQ (Scalar Binary Quantization) Storage Query Execution

**Characteristics**:
- Quantized distance calculations for speed
- Resort phase with full precision for accuracy
- Lower memory usage, optimized performance

**Query Path**:
```rust
StorageState::SbqSpeedup(quantizer, iter) => {
    let bq = SbqSpeedupStorage::load_for_search(&indexrel, &heaprel, quantizer, &state.meta_page);
    let next = iter.next_with_resort(&scan, &indexrel, &bq);
    get_tuple(state, next, scan)
}
```

**SBQ Query Optimization**:
- **Quantized Search**: Fast approximate distance calculations during graph traversal
- **Resort Buffer**: Collects candidates using quantized distances
- **Full Precision Resort**: Re-ranks final candidates with exact distances

## Graph Search Algorithm

### Core Algorithm: Streaming Greedy Search

The graph search implementation uses a streaming version of the greedy search algorithm:

```rust
// File: src/access_method/graph/mod.rs
pub fn greedy_search_iterate<S: Storage>(
    &self,
    lsr: &mut ListSearchResult<S::QueryDistanceMeasure, S::LSNPrivateData>,
    visit_n_closest: usize,
    no_filter: bool,
    mut visited_nodes: Option<&mut HashSet<NeighborWithDistance>>,
    storage: &S,
)
```

### Data Structures

**ListSearchResult**: Manages the search state
```rust
struct ListSearchResult<QDM, PD> {
    candidates: BinaryHeap<Reverse<ListSearchNeighbor<PD>>>, // Priority queue of candidates
    visited: Vec<ListSearchNeighbor<PD>>,                    // Visited nodes (results)
    inserted: HashSet<ItemPointer>,                          // Prevents duplicate visits
    sdm: Option<QDM>,                                        // Query distance measure
    stats: GreedySearchStats,                                // Performance statistics
}
```

**TSVResponseIterator**: Manages result streaming
```rust
struct TSVResponseIterator<QDM, PD> {
    lsr: ListSearchResult<QDM, PD>,           // Core search state
    search_list_size: usize,                  // Beam width parameter
    resort_size: usize,                       // Size of resort buffer
    resort_buffer: BinaryHeap<ResortData>,    // Resort candidates
    has_label_filter: bool,                   // Whether query has label filtering
    // ... statistics and counters
}
```

### Search Algorithm Steps

1. **Initialization**:
   - Start from entry nodes (selected based on labels if applicable)
   - Initialize priority queues and visited sets
   - Set up distance measures

2. **Iterative Expansion**:
   - Select closest unvisited node from candidates
   - Visit all neighbors of selected node
   - Apply label filtering during neighbor processing
   - Add valid neighbors to candidate queue
   - Mark current node as visited and add to results

3. **Result Streaming**:
   - Return visited nodes in distance order
   - Handle deleted tuples by skipping and continuing search
   - Maintain buffer pins for PostgreSQL locking requirements

4. **Termination**:
   - Continue until no more candidates or LIMIT reached
   - Handle early termination gracefully

### Label-Aware Start Node Selection

When label filtering is active, the algorithm selects appropriate start nodes:

```rust
let start_nodes = if no_filter {
    start_nodes.unwrap().get_for_node(None)
} else {
    start_nodes.unwrap().get_for_node(query.labels())
};
```

This optimization ensures that the search begins from nodes that are likely to lead to results matching the label criteria.

## Performance Optimizations

### Resort Buffer (SBQ Storage)

For SBQ storage, results undergo a two-phase optimization:

1. **Quantized Search Phase**: Fast graph traversal using approximate distances
2. **Full Precision Resort Phase**: Re-rank top candidates with exact distances

```rust
fn next_with_resort<S: Storage<QueryDistanceMeasure = QDM, LSNPrivateData = PD>>(
    &mut self,
    scan: &PgBox<pg_sys::IndexScanDescData>,
    _index: &PgRelation,
    storage: &S,
) -> Option<(HeapPointer, IndexPointer)>
```

**Resort Buffer Configuration**:
- Size controlled by `TSV_RESORT_SIZE` GUC parameter
- Collects candidates using fast quantized distances
- Applies full precision re-ranking before returning results

### Lazy Deletion Detection

Deleted tuples are detected during query execution rather than immediately removed:

```rust
// In TSVResponseIterator::next()
loop {
    let item = self.lsr.consume(storage);
    match item {
        Some((heap_pointer, index_pointer)) => {
            if heap_pointer.offset == InvalidOffsetNumber {
                continue; // Skip deleted tuple
            }
            return Some((heap_pointer, index_pointer));
        }
        None => return None,
    }
}
```

This approach avoids expensive index maintenance during delete operations.

### Memory Management

- **Scan State**: Allocated in PostgreSQL's memory context system
- **Buffer Pinning**: Maintains pins on index pages per PostgreSQL requirements
- **Streaming**: Results are generated on-demand without materializing full result sets

### Statistics and Monitoring

The system tracks detailed performance metrics:

```rust
// Logged at scan completion
debug1!(
    "Query stats - reads_index={} reads_heap={} d_total={} d_quantized={} d_full={} next={} resort={} visits={} candidate={}",
    iter.lsr.stats.get_node_reads(),
    iter.lsr.stats.get_node_heap_reads(),
    iter.lsr.stats.get_total_distance_comparisons(),
    iter.lsr.stats.get_quantized_distance_comparisons(),
    iter.full_distance_comparisons,
    iter.next_calls,
    iter.next_calls_with_resort,
    iter.lsr.stats.get_visited_nodes(),
    iter.lsr.stats.get_candidate_nodes(),
);
```

## Differences Between Labeled and Unlabeled Indexes

### Unlabeled Index Query Execution

**Characteristics**:
- Simpler query path without label filtering logic
- All nodes in graph are potential candidates
- Start nodes selected without label considerations

**Query Flow**:
1. Extract only vector from ORDER BY clause
2. Use global start nodes for graph entry
3. Visit all neighbors during graph traversal
4. No label-based filtering during search

### Labeled Index Query Execution

**Characteristics**:
- Two-column index: `(embedding, labels)`
- Label-aware start node selection
- Label filtering integrated into graph traversal
- Support for both labeled and unlabeled queries

**Query Flow**:
1. Extract vector from ORDER BY and labels from WHERE clause
2. Choose label-appropriate start nodes if labels specified
3. Apply label filtering during neighbor visitation
4. Support queries with or without label filters on same index

**Index Creation Difference**:
```sql
-- Unlabeled index
CREATE INDEX ON documents USING diskann (embedding vector_cosine_ops);

-- Labeled index  
CREATE INDEX ON documents USING diskann (embedding vector_cosine_ops, labels);
```

### Performance Implications

- **Labeled Indexes**: Higher memory usage due to label storage, but support efficient filtering
- **Unlabeled Indexes**: Lower memory usage, simpler query path, but no label filtering optimization
- **Query Compatibility**: Labeled indexes can handle both labeled and unlabeled queries
- **Start Node Optimization**: Labeled indexes select better entry points for filtered queries

## Configuration Parameters

### GUC Parameters

- **`diskann.query_search_list_size`**: Controls the beam width for graph search
- **`tsv_resort_size`**: Size of resort buffer for SBQ queries
- **Build-time parameters**: Affect index structure but not query execution

### Performance Tuning

- **Search List Size**: Higher values increase accuracy but reduce speed
- **Resort Size**: Larger buffers improve accuracy for SBQ indexes
- **Label Selectivity**: More selective labels improve query performance

This query execution workflow provides the foundation for pgvectorscale's high-performance approximate nearest neighbor search with integrated label filtering capabilities.
