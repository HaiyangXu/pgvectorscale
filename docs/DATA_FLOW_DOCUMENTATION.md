# pgvectorscale Data Insertion and Deletion Flow

## Overview

This document explains the complete code flow for data insertion and deletion in pgvectorscale, including how data is stored, indexed, and removed from the DiskANN graph structure.

## Data Insertion Flow

### 1. Entry Point: PostgreSQL Access Method Interface

When PostgreSQL needs to insert a new tuple into a DiskANN index, it calls the access method handler:

```rust
// File: src/access_method/build.rs
#[pg_guard]
pub unsafe extern "C" fn aminsert(
    indexrel: pg_sys::Relation,
    values: *mut pg_sys::Datum,
    isnull: *mut bool,
    heap_tid: pg_sys::ItemPointer,
    heaprel: pg_sys::Relation,
    _check_unique: pg_sys::IndexUniqueCheck::Type,
    _index_unchanged: bool,
    _index_info: *mut pg_sys::IndexInfo,
) -> bool {
    aminsert_internal(indexrel, values, isnull, heap_tid, heaprel)
}
```

### 2. Vector Extraction and Validation

The internal insert function extracts and validates the vector data:

```rust
// File: src/access_method/build.rs
unsafe fn aminsert_internal(
    indexrel: pg_sys::Relation,
    values: *mut pg_sys::Datum,
    isnull: *mut bool,
    heap_tid: pg_sys::ItemPointer,
    heaprel: pg_sys::Relation,
) -> bool {
    let index_relation = PgRelation::from_pg(indexrel);
    let heap_relation = PgRelation::from_pg(heaprel);
    let mut meta_page = MetaPage::fetch(&index_relation);

    // Extract vector from PostgreSQL datum
    let vec = LabeledVector::from_datums(values, isnull, &meta_page);
    if vec.is_none() {
        // Handle NULL vectors
        return false;
    }
    let vec = vec.unwrap();

    // Create spare copy for dual insertion (labeled vs unlabeled paths)
    let spare_vec = LabeledVector::from_datums(values, isnull, &meta_page).unwrap();
    let heap_pointer = ItemPointer::with_item_pointer_data(*heap_tid);

    // Route to appropriate storage implementation
    let mut storage = meta_page.get_storage_type();
    let mut stats = InsertStats::new();
    
    match &mut storage {
        StorageType::Plain => {
            // Use Plain storage for full precision
        }
        StorageType::SbqCompression => {
            // Use SBQ storage for memory efficiency
        }
    }
}
```

### 3. Storage Layer Routing

Based on the index configuration, the insertion is routed to either Plain or SBQ storage:

#### Plain Storage Path
```rust
// File: src/access_method/build.rs
StorageType::Plain => {
    let plain = PlainStorage::load_for_insert(
        &index_relation,
        &heap_relation,
        meta_page.get_distance_function(),
    );
    assert!(vec.labels().is_none()); // Plain storage doesn't support labels
    insert_storage(
        &plain,
        &index_relation,
        vec,
        spare_vec,
        heap_pointer,
        &mut meta_page,
        &mut stats,
    );
}
```

#### SBQ Storage Path
```rust
// File: src/access_method/build.rs
StorageType::SbqCompression => {
    let bq = SbqSpeedupStorage::load_for_insert(
        &heap_relation,
        &index_relation,
        &meta_page,
        &mut stats.quantizer_stats,
    );
    insert_storage(
        &bq,
        &index_relation,
        vec,
        spare_vec,
        heap_pointer,
        &mut meta_page,
        &mut stats,
    );
}
```

### 4. Node Creation in Storage Layer

The `insert_storage` function handles the actual node creation and graph insertion:

```rust
// File: src/access_method/build.rs
unsafe fn insert_storage<S: Storage>(
    storage: &S,
    index_relation: &PgRelation,
    vector: LabeledVector,
    spare_vector: LabeledVector,
    heap_pointer: ItemPointer,
    meta_page: &mut MetaPage,
    stats: &mut InsertStats,
) {
    // 1. Initialize tape for writing pages
    let mut tape = Tape::resume(index_relation, S::page_type());
    
    // 2. Create the node in storage
    let index_pointer = storage.create_node(
        vector.vec().to_index_slice(),
        vector.labels().cloned(),
        heap_pointer,
        meta_page,
        &mut tape,
        stats,
    );

    // 3. Insert into graph structure
    let mut graph = Graph::new(GraphNeighborStore::Disk, meta_page);
    graph.insert(
        index_relation,
        index_pointer,
        vector,
        spare_vector,
        storage,
        stats,
    );
}
```

### 5. Storage-Specific Node Creation

#### Plain Storage Node Creation
```rust
// File: src/access_method/plain/storage.rs
fn create_node<S: StatsNodeWrite>(
    &self,
    full_vector: &[f32],
    _labels: Option<LabelSet>,
    heap_pointer: HeapPointer,
    meta_page: &MetaPage,
    tape: &mut Tape,
    stats: &mut S,
) -> ItemPointer {
    // Create PlainNode with full precision vector
    let node = PlainNode::new_for_full_vector(
        full_vector.to_vec(), 
        heap_pointer, 
        meta_page
    );
    
    // Write to PostgreSQL page using tape
    let index_pointer: IndexPointer = node.write(tape, stats);
    index_pointer
}
```

#### SBQ Storage Node Creation
```rust
// File: src/access_method/sbq/storage.rs
fn create_node<S: StatsNodeWrite>(
    &self,
    full_vector: &[f32],
    labels: Option<LabelSet>,
    heap_pointer: HeapPointer,
    meta_page: &MetaPage,
    tape: &mut Tape,
    stats: &mut S,
) -> ItemPointer {
    // Quantize the vector using SBQ
    let bq_vector = self.quantizer.vector_for_new_node(meta_page, full_vector);

    // Create SBQ node (Classic or Labeled)
    let node = SbqNode::with_meta(
        heap_pointer, 
        meta_page, 
        bq_vector.as_slice(), 
        labels
    );

    // Write quantized node to page
    let index_pointer: IndexPointer = node.write(tape, stats);
    index_pointer
}
```

### 6. Graph Insertion Process

The graph insertion handles the DiskANN algorithm implementation:

```rust
// File: src/access_method/graph/mod.rs
pub fn insert<S: Storage>(
    &mut self,
    index: &PgRelation,
    index_pointer: IndexPointer,
    vec: LabeledVector,
    spare_vec: LabeledVector,
    storage: &S,
    stats: &mut InsertStats,
) {
    // Update start nodes for navigation
    self.update_start_nodes(index, index_pointer, &vec, storage, stats);

    if vec.labels().is_some() {
        // Insert with label filtering
        self.insert_internal(index_pointer, spare_vec, false, storage, stats);
    }

    // Insert without label filtering (main path)
    self.insert_internal(index_pointer, vec, true, storage, stats);
}
```

### 7. Internal Graph Insertion Algorithm

```rust
// File: src/access_method/graph/mod.rs
fn insert_internal<S: Storage>(
    &mut self,
    index_pointer: IndexPointer,
    vec: LabeledVector,
    no_filter: bool,
    storage: &S,
    stats: &mut InsertStats,
) {
    let labels = vec.labels().cloned();

    // 1. Perform greedy search to find closest neighbors
    let v = self.greedy_search_for_build(
        index_pointer,
        vec,
        no_filter,
        storage,
        &mut stats.greedy_search_stats,
    );

    // 2. Add neighbors with pruning
    let (_, neighbor_list) = self.add_neighbors(
        storage,
        index_pointer,
        labels.as_ref(),
        v.into_iter().collect(),
        &mut stats.prune_neighbor_stats,
    );

    // 3. Update back pointers (bidirectional edges)
    let mut cnt_contains = 0;
    let neighbor_list_len = neighbor_list.len();
    for neighbor in neighbor_list {
        let neighbor_contains_new_point = self.update_back_pointer(
            neighbor.get_index_pointer_to_neighbor(),
            index_pointer,
            neighbor.get_labels(),
            labels.as_ref(),
            neighbor.get_distance_with_tie_break(),
            storage,
            &mut stats.prune_neighbor_stats,
        );
        if neighbor_contains_new_point {
            cnt_contains += 1;
        }
    }

    // 4. Check for orphaned nodes (should be rare)
    if neighbor_list_len > 0 && cnt_contains == 0 {
        pgrx::warning!("Inserted {:?} but it became an orphan", index_pointer);
    }
}
```

### 8. Page Management and Persistence

Throughout the insertion process, data is written to PostgreSQL pages:

- **Tape Writing**: Manages writing data to 8KB PostgreSQL pages
- **Buffer Management**: Integrates with PostgreSQL's shared buffer pool
- **WAL Logging**: All changes are logged for crash recovery
- **MVCC Integration**: Respects transaction isolation levels

## Data Deletion Flow

### 1. Entry Point: VACUUM Operations

Data deletion in pgvectorscale happens primarily through PostgreSQL's VACUUM process:

```rust
// File: src/access_method/vacuum.rs
#[pg_guard]
pub extern "C" fn ambulkdelete(
    info: *mut pg_sys::IndexVacuumInfo,
    stats: *mut pg_sys::IndexBulkDeleteResult,
    callback: pg_sys::IndexBulkDeleteCallback,
    callback_state: *mut ::std::os::raw::c_void,
) -> *mut pg_sys::IndexBulkDeleteResult {
    let results = if stats.is_null() {
        unsafe { PgBox::<pg_sys::IndexBulkDeleteResult>::alloc0().into_pg() }
    } else {
        stats
    };

    let index_relation = unsafe { PgRelation::from_pg((*info).index) };
    let nblocks = unsafe {
        pg_sys::RelationGetNumberOfBlocksInFork(
            index_relation.as_ptr(),
            pg_sys::ForkNumber::MAIN_FORKNUM,
        )
    };

    // Route to storage-specific deletion
    let meta_page = MetaPage::fetch(&index_relation);
    let storage = meta_page.get_storage_type();
    match storage {
        StorageType::SbqCompression => {
            // Handle SBQ node deletion
        }
        StorageType::Plain => {
            // Handle Plain node deletion
        }
    }
    results
}
```

### 2. Storage-Specific Bulk Deletion

The deletion process scans all pages and marks deleted tuples:

```rust
// File: src/access_method/vacuum.rs
fn bulk_delete_for_storage<S: Storage, N: NodeVacuum>(
    index: &PgRelation,
    nblocks: u32,
    results: *mut IndexBulkDeleteResult,
    callback: pg_sys::IndexBulkDeleteCallback,
    callback_state: *mut ::std::os::raw::c_void,
) {
    for block_number in 0..nblocks {
        let page = unsafe { WritablePage::cleanup(index, block_number) };
        
        // Skip non-relevant page types
        if page.get_type() != S::page_type() {
            continue;
        }
        
        let mut modified = false;
        let max_offset = unsafe { PageGetMaxOffsetNumber(*page) };
        
        // Process each item on the page
        for offset_number in FirstOffsetNumber..(max_offset + 1) as _ {
            unsafe {
                let item_id = PageGetItemId(*page, offset_number);
                let item = PageGetItem(*page, item_id) as *mut u8;
                let len = (*item_id).lp_len();
                let data = std::slice::from_raw_parts_mut(item, len as _);
                let node = N::with_data(data);

                // Skip already deleted nodes
                if node.is_deleted() {
                    continue;
                }

                // Check if heap tuple should be deleted
                let heap_pointer: ItemPointer = node.get_heap_item_pointer();
                let mut ctid: pg_sys::ItemPointerData = pg_sys::ItemPointerData {
                    ..Default::default()
                };
                heap_pointer.to_item_pointer_data(&mut ctid);

                let deleted = callback.unwrap()(&mut ctid, callback_state);
                if deleted {
                    // Mark node as deleted
                    N::delete(node);
                    modified = true;
                    (*results).tuples_removed += 1.0;
                } else {
                    (*results).num_index_tuples += 1.0;
                }
            }
        }
        
        // Commit page changes if modified
        if modified {
            page.commit();
        }
    }
}
```

### 3. Node Deletion Implementation

#### Plain Node Deletion
```rust
// File: src/access_method/plain/node.rs
impl NodeVacuum for ArchivedPlainNode {
    fn delete(self: Pin<&mut Self>) {
        // Mark as deleted by setting invalid offset
        let mut heap_pointer = unsafe { 
            self.map_unchecked_mut(|s| &mut s.heap_item_pointer) 
        };
        heap_pointer.offset = InvalidOffsetNumber;
        heap_pointer.block_number = InvalidBlockNumber;
    }
}
```

#### SBQ Node Deletion
```rust
// File: src/access_method/sbq/node.rs
impl NodeVacuum for ArchivedClassicSbqNode {
    fn delete(self: Pin<&mut Self>) {
        // Mark as deleted by setting invalid offset
        let mut heap_pointer = unsafe { 
            self.map_unchecked_mut(|s| &mut s.heap_item_pointer) 
        };
        heap_pointer.offset = InvalidOffsetNumber;
        heap_pointer.block_number = InvalidBlockNumber;
    }
}
```

### 4. Deletion Detection During Queries

During query processing, deleted nodes are detected and skipped:

```rust
// File: src/access_method/scan.rs
impl<QDM, PD> TSVResponseIterator<QDM, PD> {
    fn next<S: Storage<QueryDistanceMeasure = QDM, LSNPrivateData = PD>>(
        &mut self,
        storage: &S,
    ) -> Option<(HeapPointer, IndexPointer)> {
        let graph = Graph::new(GraphNeighborStore::Disk, &mut self.meta_page);

        // Iterate until we find a non-deleted tuple
        loop {
            graph.greedy_search_iterate(
                &mut self.lsr,
                self.search_list_size,
                !self.has_label_filter,
                None,
                storage,
            );

            let item = self.lsr.consume(storage);

            match item {
                Some((heap_pointer, index_pointer)) => {
                    if heap_pointer.offset == InvalidOffsetNumber {
                        // Skip deleted tuple
                        continue;
                    }
                    return Some((heap_pointer, index_pointer));
                }
                None => {
                    return None;
                }
            }
        }
    }
}
```

## Key Design Considerations

### 1. Lazy Deletion Strategy

pgvectorscale implements **lazy deletion** - nodes are marked as deleted but not immediately removed from the graph structure. This design choice has several implications:

- **Performance**: Avoids expensive graph restructuring during deletion
- **Consistency**: Maintains graph connectivity during concurrent operations
- **Recovery**: Simplifies crash recovery since graph structure remains stable

### 2. Graph Connectivity Preservation

When nodes are deleted:
- The graph structure remains intact (edges are not immediately updated)
- During queries, deleted nodes are detected and skipped
- The graph remains navigable even with deleted nodes
- Future VACUUM operations may optimize the graph structure

### 3. Storage Layer Abstraction

The deletion flow demonstrates the clean separation between:
- **Access Method Interface**: PostgreSQL integration
- **Storage Layer**: Plain vs SBQ implementations
- **Graph Layer**: DiskANN algorithm logic
- **Page Management**: PostgreSQL page and buffer integration

### 4. Transaction Integration

Both insertion and deletion respect PostgreSQL's ACID properties:
- **Atomicity**: All changes within a transaction are atomic
- **Consistency**: Graph invariants are maintained
- **Isolation**: MVCC ensures concurrent access safety
- **Durability**: WAL logging ensures persistence

## Performance Characteristics

### Insertion Performance
- **Plain Storage**: O(log n) for storage + O(k × log n) for graph insertion
- **SBQ Storage**: O(d) for quantization + O(k × log n) for graph insertion
- **Graph Updates**: Bidirectional edge updates require neighbor list modifications

### Deletion Performance
- **Marking Phase**: O(1) per node (just sets invalid offset)
- **Query Impact**: Deleted nodes add minimal overhead during search
- **Space Reclamation**: Happens during VACUUM operations

This flow demonstrates how pgvectorscale efficiently manages vector data while maintaining the complex graph structure required for approximate nearest neighbor search.
