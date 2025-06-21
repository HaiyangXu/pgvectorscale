# Bitmap Scan Support Implementation for pgvectorscale

## Overview

This implementation adds bitmap scan support to pgvectorscale, enabling PostgreSQL's query planner to use multiple indexes together in complex queries involving vector similarity search.

## What are Bitmap Scans?

Bitmap scans are a PostgreSQL technique that allows using multiple indexes simultaneously:

1. **Individual Index Scans**: Each index scan creates a bitmap of matching row identifiers (TIDs)
2. **Bitmap Combination**: Multiple bitmaps are combined using logical operations (AND, OR, NOT)
3. **Heap Access**: The final combined bitmap is used to efficiently access the heap table

## Implementation Details

### Files Modified

1. **`src/access_method/scan.rs`**:
   - Added `amgetbitmap` function that implements the bitmap scan interface
   - Function signature: `extern "C" fn amgetbitmap(scan: IndexScanDesc, tbm: *mut TIDBitmap) -> i64`
   - Collects all matching tuples and adds them to the provided TIDBitmap
   - Supports both `SbqSpeedup` and `Plain` storage types

2. **`src/access_method/mod.rs`**:
   - Updated access method handler to enable bitmap scans
   - Set `amroutine.amgetbitmap = Some(scan::amgetbitmap)`

### Key Implementation Points

1. **Tuple Collection**: The `amgetbitmap` function iterates through all matching results using the existing `next()` and `next_with_resort()` methods

2. **Bitmap Population**: Each matching tuple's heap pointer is converted to `ItemPointerData` and added to the bitmap using `pg_sys::tbm_add_tuples()`

3. **Storage Type Support**: Handles both storage implementations:
   - `SbqSpeedup`: Uses quantized vector search with speedup techniques
   - `Plain`: Uses direct vector comparison without quantization

4. **Return Value**: Returns the count of tuples added to the bitmap

## How Bitmap Scans Work with Vector Ordering

For a query like:
```sql
SELECT * FROM users 
WHERE age > 25 AND city = 'New York' AND category_id IN (1, 2, 3) 
ORDER BY embedding <=> '[1,2,3]' 
LIMIT 10;
```

**The execution is:**
1. **Bitmap Index Scan on age index** → creates bitmap1 for `age > 25`
2. **Bitmap Index Scan on city index** → creates bitmap2 for `city = 'New York'`
3. **Bitmap Index Scan on category index** → creates bitmap3 for `category_id IN (1, 2, 3)`
4. **BitmapAnd node** → combines: `final_bitmap = bitmap1 AND bitmap2 AND bitmap3`
5. **BitmapHeapScan node** → reads heap tuples using `final_bitmap`, computes vector distances for each qualifying tuple, and sorts by distance

**Key Point**: The vector index is **not used for bitmap creation** in this scenario. It's only used if there are vector-specific WHERE conditions like `embedding <=> '[1,2,3]' < 0.5`.

Our `amgetbitmap` implementation enables vector indexes to participate in bitmap scans when there ARE vector WHERE conditions, but for pure ORDER BY clauses, the ordering happens during heap scanning.

## When Vector Index Bitmap Scan IS Used

The vector index `amgetbitmap` function would be called in queries with vector WHERE conditions:

```sql
-- Vector index WILL participate in bitmap scan
SELECT * FROM users 
WHERE age > 25 
  AND city = 'New York' 
  AND embedding <=> '[1,2,3]' < 0.5  -- This triggers vector bitmap scan
ORDER BY embedding <=> '[1,2,3]'
```

**Execution plan:**
1. **Bitmap Index Scan on age index** → bitmap1
2. **Bitmap Index Scan on city index** → bitmap2  
3. **Bitmap Index Scan on vector index** → bitmap3 (calls our `amgetbitmap`)
4. **BitmapAnd** → `final_bitmap = bitmap1 AND bitmap2 AND bitmap3`
5. **BitmapHeapScan** → reads heap using combined bitmap and sorts by distance

## Benefits

1. **Multi-Index Usage**: Can combine vector similarity with traditional filtering conditions
2. **Performance**: More efficient than sequential scans for complex queries
3. **Flexibility**: Query planner can choose optimal execution strategy
4. **Compatibility**: Integrates seamlessly with PostgreSQL's existing bitmap scan infrastructure

## Technical Notes

- Bitmap scans don't guarantee tuple ordering (unlike `amgettuple`)
- More efficient for bulk operations as they avoid repeated lock/unlock cycles
- The implementation reuses existing scan infrastructure for consistency
- Works with PostgreSQL's cost-based optimizer to choose optimal plans

## Testing

The implementation can be tested using the provided `bitmap_scan_test.sql` file, which includes:
- Multi-index table setup
- Various complex queries combining vector search with filtering
- Query plan analysis with `EXPLAIN ANALYZE`

## Future Enhancements

1. **Cost Estimation**: May need refinement for bitmap vs tuple scan costs
2. **Performance Optimization**: Could optimize bitmap collection for very large result sets
3. **Parallel Support**: Future versions could add parallel bitmap scan support
4. **Statistics Integration**: Enhanced statistics for better query planning
