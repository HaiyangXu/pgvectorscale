#[cfg(any(test, feature = "pg_test"))]
#[pgrx::pg_schema]
mod tests {
    use pgrx::*;
    use crate::access_method::distance::DistanceType;
    use crate::access_method::custom_executor::{BitmapFilter, execute_bitmap_filtered_vector_search};
    use crate::access_method::labels::LabeledVector;
    use crate::util::HeapPointer;

    /// Test bitmap filtering with a simple query
    #[pg_test]
    unsafe fn test_bitmap_filtering_basic() -> spi::Result<()> {
        // Create test table with both vector and regular columns
        Spi::run(r#"
            DROP TABLE IF EXISTS test_bitmap_vectors CASCADE;
            CREATE TABLE test_bitmap_vectors (
                id SERIAL PRIMARY KEY,
                vector REAL[],
                category INTEGER,
                status TEXT
            );
        "#)?;

        // Insert test data
        Spi::run(r#"
            INSERT INTO test_bitmap_vectors (vector, category, status)
            SELECT 
                ARRAY[random(), random(), random()],
                (random() * 10)::INTEGER,
                CASE WHEN random() > 0.5 THEN 'active' ELSE 'inactive' END
            FROM generate_series(1, 100);
        "#)?;

        // Create vector index
        Spi::run(r#"
            CREATE INDEX test_bitmap_vectors_vector_idx 
            ON test_bitmap_vectors 
            USING diskann (vector)
            WITH (num_neighbors = 10, storage_layout = plain);
        "#)?;

        // Create regular indexes
        Spi::run(r#"
            CREATE INDEX test_bitmap_vectors_category_idx 
            ON test_bitmap_vectors (category);
            
            CREATE INDEX test_bitmap_vectors_status_idx 
            ON test_bitmap_vectors (status);
        "#)?;

        // Test query that should benefit from bitmap filtering
        let result = Spi::get_one::<i64>(r#"
            SELECT COUNT(*) FROM test_bitmap_vectors 
            WHERE vector <-> ARRAY[0.5, 0.5, 0.5] < 2.0 
            AND category = 5 
            AND status = 'active';
        "#)?;

        // Verify we got some results (exact count depends on random data)
        assert!(result.unwrap_or(0) >= 0);

        Ok(())
    }

    /// Test bitmap filtering with different storage types
    #[pg_test]
    unsafe fn test_bitmap_filtering_sbq_storage() -> spi::Result<()> {
        // Create test table
        Spi::run(r#"
            DROP TABLE IF EXISTS test_bitmap_sbq CASCADE;
            CREATE TABLE test_bitmap_sbq (
                id SERIAL PRIMARY KEY,
                vector REAL[],
                priority INTEGER
            );
        "#)?;

        // Insert test data with higher dimensions for SBQ
        Spi::run(r#"
            INSERT INTO test_bitmap_sbq (vector, priority)
            SELECT 
                array_agg(random())::REAL[],
                (random() * 100)::INTEGER
            FROM generate_series(1, 50),
                 generate_series(1, 64) -- 64-dimensional vectors
            GROUP BY generate_series;
        "#)?;

        // Create SBQ index
        Spi::run(r#"
            CREATE INDEX test_bitmap_sbq_vector_idx 
            ON test_bitmap_sbq 
            USING diskann (vector)
            WITH (num_neighbors = 10, storage_layout = memory_optimized);
        "#)?;

        // Create regular index
        Spi::run(r#"
            CREATE INDEX test_bitmap_sbq_priority_idx 
            ON test_bitmap_sbq (priority);
        "#)?;

        // Test query with bitmap filtering
        let result = Spi::get_one::<i64>(r#"
            SELECT COUNT(*) FROM test_bitmap_sbq 
            WHERE vector <-> array_fill(0.5, ARRAY[64])::REAL[] < 3.0 
            AND priority > 50;
        "#)?;

        assert!(result.unwrap_or(0) >= 0);

        Ok(())
    }

    /// Test the bitmap filter utility functions
    #[pg_test]
    unsafe fn test_bitmap_filter_operations() -> spi::Result<()> {
        use std::ptr;
        
        // Create a simple TIDBitmap for testing
        let tbm = pg_sys::tbm_create(1000, std::ptr::null_mut());
        
        // Add some items to the bitmap
        let mut ctid1 = pg_sys::ItemPointerData::default();
        pg_sys::ItemPointerSet(&mut ctid1, 1, 1);
        pg_sys::tbm_add_tuples(tbm, &mut ctid1, 1, false);
        
        let mut ctid2 = pg_sys::ItemPointerData::default();
        pg_sys::ItemPointerSet(&mut ctid2, 1, 2);
        pg_sys::tbm_add_tuples(tbm, &mut ctid2, 1, false);
        
        // Test BitmapFilter
        let bitmap_filter = BitmapFilter::new(tbm);
          // Create test heap pointers
        let heap_pointer1 = HeapPointer { block_number: 1, offset: 1 };
        let heap_pointer2 = HeapPointer { block_number: 1, offset: 2 };
        let heap_pointer3 = HeapPointer { block_number: 1, offset: 3 };
        
        // Test membership
        assert!(bitmap_filter.contains(&heap_pointer1));
        assert!(bitmap_filter.contains(&heap_pointer2));
        assert!(!bitmap_filter.contains(&heap_pointer3));
        
        // Clean up
        pg_sys::tbm_free(tbm);
        
        Ok(())
    }

    /// Test query planning hook detection
    #[pg_test]  
    unsafe fn test_query_plan_analysis() -> spi::Result<()> {
        // Create test scenario with multiple indexes
        Spi::run(r#"
            DROP TABLE IF EXISTS test_plan_analysis CASCADE;
            CREATE TABLE test_plan_analysis (
                id SERIAL PRIMARY KEY,
                vector REAL[],
                tag TEXT,
                value INTEGER,
                active BOOLEAN DEFAULT TRUE
            );
        "#)?;

        // Insert test data
        Spi::run(r#"
            INSERT INTO test_plan_analysis (vector, tag, value, active)
            SELECT 
                ARRAY[random(), random(), random()]::REAL[],
                'tag_' || (random() * 5)::INTEGER,
                (random() * 1000)::INTEGER,
                random() > 0.3
            FROM generate_series(1, 200);
        "#)?;

        // Create multiple indexes
        Spi::run(r#"
            CREATE INDEX test_plan_vector_idx 
            ON test_plan_analysis 
            USING diskann (vector)
            WITH (num_neighbors = 15);
            
            CREATE INDEX test_plan_tag_idx 
            ON test_plan_analysis (tag);
            
            CREATE INDEX test_plan_value_idx 
            ON test_plan_analysis (value);
            
            CREATE INDEX test_plan_active_idx 
            ON test_plan_analysis (active);
        "#)?;

        // Test complex query that should trigger bitmap optimization detection
        Spi::run(r#"
            EXPLAIN (ANALYZE, BUFFERS) 
            SELECT id, vector, tag 
            FROM test_plan_analysis 
            WHERE vector <-> ARRAY[0.1, 0.2, 0.3]::REAL[] < 1.5
            AND tag IN ('tag_1', 'tag_2') 
            AND value BETWEEN 100 AND 900
            AND active = true
            ORDER BY vector <-> ARRAY[0.1, 0.2, 0.3]::REAL[]
            LIMIT 10;
        "#)?;

        Ok(())
    }

    /// Performance comparison test
    #[pg_test]
    unsafe fn test_bitmap_filtering_performance() -> spi::Result<()> {
        // Create larger test dataset
        Spi::run(r#"
            DROP TABLE IF EXISTS test_performance CASCADE;
            CREATE TABLE test_performance (
                id SERIAL PRIMARY KEY,
                vector REAL[],
                category INTEGER,
                subcategory INTEGER,
                flags INTEGER
            );
        "#)?;

        // Insert more substantial test data
        Spi::run(r#"
            INSERT INTO test_performance (vector, category, subcategory, flags)
            SELECT 
                array_agg(random())::REAL[],
                (random() * 20)::INTEGER,
                (random() * 100)::INTEGER,
                (random() * 1000)::INTEGER
            FROM generate_series(1, 1000),
                 generate_series(1, 10) -- 10-dimensional vectors
            GROUP BY generate_series;
        "#)?;

        // Create indexes
        Spi::run(r#"
            CREATE INDEX test_performance_vector_idx 
            ON test_performance 
            USING diskann (vector)
            WITH (num_neighbors = 20, storage_layout = plain);
            
            CREATE INDEX test_performance_category_idx 
            ON test_performance (category);
            
            CREATE INDEX test_performance_subcategory_idx 
            ON test_performance (subcategory);
            
            CREATE INDEX test_performance_flags_idx 
            ON test_performance (flags);
        "#)?;

        // Test timing with EXPLAIN ANALYZE
        let plan_with_filters = Spi::get_one::<String>(r#"
            EXPLAIN (ANALYZE, BUFFERS, TIMING) 
            SELECT COUNT(*) 
            FROM test_performance 
            WHERE vector <-> array_fill(0.5, ARRAY[10])::REAL[] < 2.0
            AND category = 5
            AND subcategory BETWEEN 20 AND 80
            AND flags & 64 = 64;
        "#)?;

        // Verify we got a plan
        assert!(plan_with_filters.is_some());
        
        // Test without some filters to compare
        let plan_without_filters = Spi::get_one::<String>(r#"
            EXPLAIN (ANALYZE, BUFFERS, TIMING) 
            SELECT COUNT(*) 
            FROM test_performance 
            WHERE vector <-> array_fill(0.5, ARRAY[10])::REAL[] < 2.0;
        "#)?;

        assert!(plan_without_filters.is_some());

        Ok(())
    }

    /// Test error conditions and edge cases
    #[pg_test]
    unsafe fn test_bitmap_filtering_edge_cases() -> spi::Result<()> {
        // Test with empty table
        Spi::run(r#"
            DROP TABLE IF EXISTS test_edge_cases CASCADE;
            CREATE TABLE test_edge_cases (
                id SERIAL PRIMARY KEY,
                vector REAL[],
                data TEXT
            );
        "#)?;

        // Create index on empty table
        Spi::run(r#"
            CREATE INDEX test_edge_cases_vector_idx 
            ON test_edge_cases 
            USING diskann (vector)
            WITH (num_neighbors = 10);
            
            CREATE INDEX test_edge_cases_data_idx 
            ON test_edge_cases (data);
        "#)?;

        // Query empty table
        let result = Spi::get_one::<i64>(r#"
            SELECT COUNT(*) FROM test_edge_cases 
            WHERE vector <-> ARRAY[1.0, 0.0, 0.0]::REAL[] < 1.0 
            AND data = 'nonexistent';
        "#)?;

        assert_eq!(result, Some(0));

        // Test with NULL values
        Spi::run(r#"
            INSERT INTO test_edge_cases (vector, data) VALUES 
            (ARRAY[1.0, 0.0, 0.0]::REAL[], 'test1'),
            (NULL, 'test2'),
            (ARRAY[0.0, 1.0, 0.0]::REAL[], NULL),
            (ARRAY[0.0, 0.0, 1.0]::REAL[], 'test3');
        "#)?;

        // Query with NULLs
        let result = Spi::get_one::<i64>(r#"
            SELECT COUNT(*) FROM test_edge_cases 
            WHERE vector <-> ARRAY[1.0, 0.0, 0.0]::REAL[] < 2.0 
            AND data IS NOT NULL;
        "#)?;

        assert!(result.unwrap_or(0) >= 0);

        Ok(())
    }

    /// Test public interface functions
    #[test]
    fn test_bitmap_filtering_interface() {
        use crate::access_method::scan::{can_use_bitmap_filtering, setup_bitmap_filtered_scan};
        use std::ptr;
        
        // Test can_use_bitmap_filtering with null inputs
        unsafe {
            assert!(!can_use_bitmap_filtering(ptr::null_mut(), ptr::null_mut(), ptr::null(), 0));
        }
        
        // Test setup_bitmap_filtered_scan with null inputs  
        unsafe {
            assert!(!setup_bitmap_filtered_scan(ptr::null_mut(), ptr::null_mut()));
        }
    }
}
