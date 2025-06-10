/*!
 * Custom Executor Extension for Bitmap-Filtered Vector Search
 * 
 * This module implements a custom executor that integrates bitmap filtering
 * with vector search, allowing distance calculations to be filtered by 
 * bitmaps from other indexes before performing expensive vector operations.
 */

use std::collections::HashSet;
use std::ffi::{c_char, CStr};
use pgrx::{pg_sys, PgBox, PgRelation, PgMemoryContexts, void_mut_ptr};

use crate::access_method::{
    graph::{Graph, ListSearchResult},
    labels::LabeledVector,
    meta_page::MetaPage,
    storage::Storage,
    graph::neighbor_store::GraphNeighborStore,
    stats::GreedySearchStats,
};
use crate::util::{HeapPointer, IndexPointer, ItemPointer};

/// Bitmap filter structure that wraps PostgreSQL's TIDBitmap
pub struct BitmapFilter {
    tbm: *mut pg_sys::TIDBitmap,
}

impl BitmapFilter {
    /// Create a new bitmap filter from PostgreSQL's TIDBitmap
    pub unsafe fn new(tbm: *mut pg_sys::TIDBitmap) -> Self {
        Self { tbm }
    }

    /// Check if a heap pointer is present in the bitmap
    pub unsafe fn contains(&self, heap_pointer: &HeapPointer) -> bool {
        if self.tbm.is_null() {
            return true; // No filter means all tuples are allowed
        }

        let mut ctid = pg_sys::ItemPointerData::default();
        heap_pointer.to_item_pointer_data(&mut ctid);
        
        // Use PostgreSQL's tbm_is_member function to check membership
        pg_sys::tbm_is_member(self.tbm, &ctid)
    }
}

/// Enhanced TSVResponseIterator that supports bitmap filtering
pub struct BitmapFilteredTSVResponseIterator<QDM, PD> {
    lsr: ListSearchResult<QDM, PD>,
    search_list_size: usize,
    meta_page: MetaPage,
    bitmap_filter: Option<BitmapFilter>,
    filtered_count: usize,
    total_checked: usize,
}

impl<QDM, PD> BitmapFilteredTSVResponseIterator<QDM, PD> {
    /// Create a new bitmap-filtered iterator
    pub fn new<S: Storage<QueryDistanceMeasure = QDM, LSNPrivateData = PD>>(
        storage: &S,
        index: &PgRelation,
        query: LabeledVector,
        search_list_size: usize,
        meta_page: MetaPage,
        bitmap_filter: Option<BitmapFilter>,
    ) -> Self {
        let mut mp = meta_page.clone();
        let graph = Graph::new(GraphNeighborStore::Disk, &mut mp);
        let lsr = graph.greedy_search_streaming_init(query, search_list_size, storage);

        Self {
            lsr,
            search_list_size,
            meta_page,
            bitmap_filter,
            filtered_count: 0,
            total_checked: 0,
        }
    }

    /// Get the next vector result, filtering by bitmap if present
    pub fn next_filtered<S: Storage<QueryDistanceMeasure = QDM, LSNPrivateData = PD>>(
        &mut self,
        storage: &S,
    ) -> Option<(HeapPointer, IndexPointer)> {
        let mut graph = Graph::new(GraphNeighborStore::Disk, &mut self.meta_page);

        loop {
            // Advance the graph search
            graph.greedy_search_iterate(
                &mut self.lsr,
                self.search_list_size,
                true, // no_filter for now, we'll apply bitmap filter below
                None,
                storage,
            );

            // Try to consume a result
            if let Some((heap_pointer, index_pointer)) = self.lsr.consume(storage) {
                self.total_checked += 1;

                // Apply bitmap filter if present
                if let Some(ref bitmap_filter) = self.bitmap_filter {
                    if unsafe { !bitmap_filter.contains(&heap_pointer) } {
                        self.filtered_count += 1;
                        // Skip this result and continue searching
                        continue;
                    }
                }

                // Check for deleted tuples
                if heap_pointer.offset == pg_sys::InvalidOffsetNumber {
                    continue;
                }

                return Some((heap_pointer, index_pointer));
            } else {
                // No more results
                return None;
            }
        }
    }

    /// Get statistics about filtering effectiveness
    pub fn get_filter_stats(&self) -> (usize, usize) {
        (self.filtered_count, self.total_checked)
    }
}

/// Custom executor node state for bitmap-filtered vector search
pub struct BitmapFilteredVectorScanState {
    /// The vector index relation
    index_relation: *mut pg_sys::RelationData,
    /// The heap relation
    heap_relation: *mut pg_sys::RelationData,
    /// Query vector and parameters
    query: Option<LabeledVector>,
    /// Bitmap filter from other indexes
    bitmap_filter: Option<BitmapFilter>,
    /// Iterator for results
    iterator: Option<Box<dyn Iterator<Item = (HeapPointer, IndexPointer)>>>,
    /// Search parameters
    search_list_size: usize,
    /// Statistics
    tuples_returned: i64,
    tuples_filtered: i64,
}

impl BitmapFilteredVectorScanState {
    /// Create a new custom executor state
    pub fn new(
        index_relation: *mut pg_sys::RelationData,
        heap_relation: *mut pg_sys::RelationData,
        query: LabeledVector,
        search_list_size: usize,
        bitmap_filter: Option<BitmapFilter>,
    ) -> Self {
        Self {
            index_relation,
            heap_relation,
            query: Some(query),
            bitmap_filter,
            iterator: None,
            search_list_size,
            tuples_returned: 0,
            tuples_filtered: 0,
        }
    }

    /// Initialize the iterator based on storage type
    pub unsafe fn initialize_iterator(&mut self) {
        if self.iterator.is_some() {
            return;
        }

        let index_rel = PgRelation::from_pg(self.index_relation);
        let heap_rel = PgRelation::from_pg(self.heap_relation);
        let meta_page = MetaPage::fetch(&index_rel);
        
        if let Some(query) = self.query.take() {
            match meta_page.get_storage_type() {
                crate::access_method::storage::StorageType::Plain => {
                    let storage = crate::access_method::plain::storage::PlainStorage::load_for_search(
                        &index_rel,
                        &heap_rel,
                        meta_page.get_distance_function(),
                    );
                    
                    let mut iter = BitmapFilteredTSVResponseIterator::new(
                        &storage,
                        &index_rel,
                        query,
                        self.search_list_size,
                        meta_page,
                        self.bitmap_filter.take(),
                    );
                    
                    // Create a boxed iterator that yields results
                    let boxed_iter = Box::new(std::iter::from_fn(move || {
                        iter.next_filtered(&storage)
                    }));
                    
                    self.iterator = Some(boxed_iter);
                }
                crate::access_method::storage::StorageType::SbqCompression => {
                    // Similar implementation for SBQ storage
                    let mut stats = crate::access_method::stats::QuantizerStats::new();
                    let quantizer = crate::access_method::sbq::SbqMeans::load(&index_rel, &meta_page, &mut stats);
                    let storage = crate::access_method::sbq::storage::SbqSpeedupStorage::load_for_search(
                        &index_rel,
                        &heap_rel,
                        &quantizer,
                        &meta_page,
                    );
                    
                    let mut iter = BitmapFilteredTSVResponseIterator::new(
                        &storage,
                        &index_rel,
                        query,
                        self.search_list_size,
                        meta_page,
                        self.bitmap_filter.take(),
                    );
                    
                    let boxed_iter = Box::new(std::iter::from_fn(move || {
                        iter.next_filtered(&storage)
                    }));
                    
                    self.iterator = Some(boxed_iter);
                }
            }
        }
    }

    /// Get the next tuple
    pub fn next_tuple(&mut self) -> Option<(HeapPointer, IndexPointer)> {
        if let Some(ref mut iterator) = self.iterator {
            if let Some(result) = iterator.next() {
                self.tuples_returned += 1;
                return Some(result);
            }
        }
        None
    }

    /// Get execution statistics
    pub fn get_stats(&self) -> (i64, i64) {
        (self.tuples_returned, self.tuples_filtered)
    }
}

/// Custom executor hook for creating bitmap-filtered vector scan plans
pub struct BitmapFilteredVectorScanHook;

impl BitmapFilteredVectorScanHook {
    /// Check if this query can benefit from bitmap-filtered vector search
    pub fn can_optimize_query(
        _root: *mut pg_sys::PlannerInfo,
        _rel: *mut pg_sys::RelOptInfo,
        _index: *mut pg_sys::IndexOptInfo,
        _clauses: *mut pg_sys::List,
    ) -> bool {
        // For now, always return false - this would be implemented
        // to detect queries that have both vector similarity conditions
        // and other conditions that could benefit from bitmap filtering
        false
    }

    /// Create a custom path for bitmap-filtered vector search
    pub unsafe fn create_custom_path(
        _root: *mut pg_sys::PlannerInfo,
        _rel: *mut pg_sys::RelOptInfo,
        _index: *mut pg_sys::IndexOptInfo,
        _clauses: *mut pg_sys::List,
    ) -> *mut pg_sys::Path {
        // This would create a custom path that represents the 
        // bitmap-filtered vector search operation
        std::ptr::null_mut()
    }
}

/// PostgreSQL custom scan methods for the bitmap-filtered vector scan
static mut BITMAP_FILTERED_VECTOR_METHODS: pg_sys::CustomScanMethods = pg_sys::CustomScanMethods {
    CustomName: b"BitmapFilteredVectorScan\0".as_ptr() as *const i8,
    CreateCustomScanState: Some(create_bitmap_filtered_scan_state),
    BeginCustomScan: None,
    ExecCustomScan: None,
    EndCustomScan: None,
    ReScanCustomScan: None,
    MarkPosCustomScan: None,
    RestrPosCustomScan: None,
    EstimateDSMCustomScan: None,
    InitializeDSMCustomScan: None,
    ReInitializeDSMCustomScan: None,
    InitializeWorkerCustomScan: None,
    ShutdownCustomScan: None,
    ExplainCustomScan: None,
};

/// Create a custom scan state for bitmap-filtered vector search
#[no_mangle]
pub unsafe extern "C" fn create_bitmap_filtered_scan_state(
    cscan: *mut pg_sys::CustomScan,
) -> *mut pg_sys::Node {
    let css = pg_sys::palloc0(std::mem::size_of::<pg_sys::CustomScanState>()) as *mut pg_sys::CustomScanState;
    
    pg_sys::NodeSetTag(css as *mut pg_sys::Node, pg_sys::NodeTag::T_CustomScanState);
    
    (*css).methods = &mut BITMAP_FILTERED_VECTOR_METHODS;
    (*css).ss.ps.type_ = pg_sys::NodeTag::T_CustomScanState;
    
    // Initialize the scan state
    (*css).ss.ps.state = (*cscan).scan.plan.state;
    (*css).ss.ps.plan = &mut (*cscan).scan.plan;
    
    // Store our custom state - we'll initialize it during execution
    (*css).custom_ps = std::ptr::null_mut();
    
    css as *mut pg_sys::Node
}

/// PostgreSQL custom scan provider for bitmap-filtered vector search  
static mut BITMAP_FILTERED_PROVIDER: pg_sys::CustomPathMethods = pg_sys::CustomPathMethods {
    CustomName: b"BitmapFilteredVectorScan\0".as_ptr() as *const i8,
    PlanCustomPath: Some(plan_bitmap_filtered_scan),
    TextOutCustomPath: None,
};

/// Plan a bitmap-filtered vector scan
#[no_mangle]
pub unsafe extern "C" fn plan_bitmap_filtered_scan(
    root: *mut pg_sys::PlannerInfo,
    rel: *mut pg_sys::RelOptInfo,
    best_path: *mut pg_sys::CustomPath,
    tlist: *mut pg_sys::List,
    clauses: *mut pg_sys::List,
    outer_plan: *mut pg_sys::Plan,
) -> *mut pg_sys::Plan {
    let cscan = pg_sys::palloc0(std::mem::size_of::<pg_sys::CustomScan>()) as *mut pg_sys::CustomScan;
    
    (*cscan).scan.plan.type_ = pg_sys::NodeTag::T_CustomScan;
    (*cscan).methods = &BITMAP_FILTERED_PROVIDER;
    
    // Set up the plan node
    (*cscan).scan.plan.targetlist = tlist;
    (*cscan).scan.plan.qual = clauses;
    (*cscan).scan.plan.lefttree = outer_plan;
    
    // Store custom data for execution
    (*cscan).custom_private = std::ptr::null_mut();
    (*cscan).custom_scan_tlist = tlist;
    
    cscan as *mut pg_sys::Plan
}

/// Register the custom executor extension with PostgreSQL
pub unsafe fn register_custom_executor() {
    // Register the custom scan provider
    pg_sys::RegisterCustomScanMethods(&BITMAP_FILTERED_PROVIDER);
}

/// Main interface function for bitmap-filtered vector search
/// This can be called from amgetbitmap when we detect that bitmap
/// filtering would be beneficial
pub unsafe fn execute_bitmap_filtered_vector_search(
    index_relation: *mut pg_sys::RelationData,
    heap_relation: *mut pg_sys::RelationData,
    query: LabeledVector,
    search_list_size: usize,
    bitmap_filter: Option<BitmapFilter>,
) -> Vec<(HeapPointer, IndexPointer)> {
    let mut state = BitmapFilteredVectorScanState::new(
        index_relation,
        heap_relation,
        query,
        search_list_size,
        bitmap_filter,
    );

    state.initialize_iterator();
    
    let mut results = Vec::new();
    while let Some(result) = state.next_tuple() {
        results.push(result);
    }
    
    results
}

/// Query planning hook for bitmap-filtered vector search
/// This function analyzes query plans to detect opportunities for bitmap filtering
pub unsafe fn query_planner_hook(
    parse: *mut pg_sys::Query,
    query_flags: i32,
    cursor_options: *mut pg_sys::ParamListInfo,
) -> *mut pg_sys::PlannedStmt {
    // Call the previous planner hook or standard planner
    let prev_hook = pg_sys::planner_hook;
    let planned_stmt = if let Some(hook) = prev_hook {
        hook(parse, query_flags, cursor_options)
    } else {
        pg_sys::standard_planner(parse, query_flags, cursor_options)
    };
    
    // Analyze the planned statement for bitmap filtering opportunities
    if !planned_stmt.is_null() {
        analyze_plan_for_bitmap_filtering((*planned_stmt).planTree);
    }
    
    planned_stmt
}

/// Analyze a plan tree to identify bitmap filtering opportunities
unsafe fn analyze_plan_for_bitmap_filtering(plan: *mut pg_sys::Plan) -> bool {
    if plan.is_null() {
        return false;
    }
    
    match (*plan).type_ {
        pg_sys::NodeTag::T_BitmapHeapScan => {
            // This is a bitmap heap scan - check if it involves vector indexes
            let bitmap_scan = plan as *mut pg_sys::BitmapHeapScan;
            analyze_bitmap_heap_scan_for_vector_optimization(bitmap_scan)
        }
        pg_sys::NodeTag::T_BitmapOr => {
            // Bitmap OR operation - recursively analyze children
            let bitmap_or = plan as *mut pg_sys::BitmapOr;
            let mut found = false;
            
            let mut cell = (*(*bitmap_or).bitmapplans).head;
            while !cell.is_null() {
                let child_plan = (*cell).data.ptr_value as *mut pg_sys::Plan;
                if analyze_plan_for_bitmap_filtering(child_plan) {
                    found = true;
                }
                cell = (*cell).next;
            }
            found
        }
        pg_sys::NodeTag::T_BitmapAnd => {
            // Bitmap AND operation - this is where our optimization shines
            let bitmap_and = plan as *mut pg_sys::BitmapAnd;
            analyze_bitmap_and_for_vector_optimization(bitmap_and)
        }
        _ => {
            // Recursively analyze child plans
            let mut found = false;
            if !(*plan).lefttree.is_null() {
                found |= analyze_plan_for_bitmap_filtering((*plan).lefttree);
            }
            if !(*plan).righttree.is_null() {
                found |= analyze_plan_for_bitmap_filtering((*plan).righttree);
            }
            found
        }
    }
}

/// Analyze a BitmapHeapScan to see if it involves vector indexes
unsafe fn analyze_bitmap_heap_scan_for_vector_optimization(
    _bitmap_scan: *mut pg_sys::BitmapHeapScan,
) -> bool {
    // For now, return false - this would be implemented to detect
    // when the bitmap scan uses vector indexes that could benefit from filtering
    false
}

/// Analyze a BitmapAnd operation for vector optimization opportunities
unsafe fn analyze_bitmap_and_for_vector_optimization(
    bitmap_and: *mut pg_sys::BitmapAnd,
) -> bool {
    let mut has_vector_index = false;
    let mut has_other_indexes = false;
    
    // Analyze each child plan in the AND operation
    let mut cell = (*(*bitmap_and).bitmapplans).head;
    while !cell.is_null() {
        let child_plan = (*cell).data.ptr_value as *mut pg_sys::Plan;
        
        if (*child_plan).type_ == pg_sys::NodeTag::T_BitmapIndexScan {
            let index_scan = child_plan as *mut pg_sys::BitmapIndexScan;
            
            // Check if this is a vector index scan
            if is_vector_index_scan(index_scan) {
                has_vector_index = true;
            } else {
                has_other_indexes = true;
            }
        }
        
        cell = (*cell).next;
    }
    
    // If we have both a vector index and other indexes in an AND operation,
    // this is a perfect candidate for bitmap filtering optimization
    has_vector_index && has_other_indexes
}

/// Check if a BitmapIndexScan is using a vector index
unsafe fn is_vector_index_scan(index_scan: *mut pg_sys::BitmapIndexScan) -> bool {
    let index_id = (*index_scan).indexid;
    
    // Get the index relation
    let index_rel = pg_sys::RelationIdGetRelation(index_id);
    if index_rel.is_null() {
        return false;
    }
    
    let index_relation = PgRelation::from_pg(index_rel);
    
    // Check if this index uses the diskann access method
    let am_oid = index_relation.rd_rel.relam;
    let am_name = pg_sys::get_am_name(am_oid);
    
    if !am_name.is_null() {
        let am_name_str = std::ffi::CStr::from_ptr(am_name);
        if let Ok(name) = am_name_str.to_str() {
            return name == "diskann";
        }
    }
    
    false
}

/// Install query planner hook for bitmap filtering optimization
pub unsafe fn install_planner_hook() {
    // Store the previous hook
    static mut PREV_PLANNER_HOOK: Option<pg_sys::planner_hook_type> = None;
    PREV_PLANNER_HOOK = pg_sys::planner_hook;
    
    // Install our hook
    pg_sys::planner_hook = Some(query_planner_hook);
}
