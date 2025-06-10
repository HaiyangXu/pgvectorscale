-- Test bitmap scan functionality for pgvectorscale
-- This demonstrates how multiple indexes can now be used together

-- First, create a test table with various columns for multi-index queries
CREATE TABLE users (
    id SERIAL PRIMARY KEY,
    age INTEGER,
    city TEXT,
    category_id INTEGER,
    embedding VECTOR(3)
);

-- Insert some test data
INSERT INTO users (age, city, category_id, embedding) VALUES
(25, 'New York', 1, '[1.0, 2.0, 3.0]'),
(30, 'New York', 2, '[2.0, 3.0, 4.0]'),
(35, 'Boston', 1, '[3.0, 4.0, 5.0]'),
(28, 'New York', 3, '[1.5, 2.5, 3.5]'),
(40, 'Boston', 2, '[4.0, 5.0, 6.0]'),
(22, 'Chicago', 1, '[0.5, 1.5, 2.5]');

-- Create various indexes for multi-index bitmap scanning
CREATE INDEX idx_users_age ON users (age);
CREATE INDEX idx_users_city ON users (city);
CREATE INDEX idx_users_category ON users (category_id);

-- Create the vector index using pgvectorscale (diskann)
CREATE INDEX idx_users_embedding ON users USING diskann (embedding vector_cosine_ops);

-- Test query that can now use bitmap scans with multiple indexes
-- The WHERE conditions will be handled by traditional indexes, 
-- and their results combined with the vector index via bitmap operations
EXPLAIN (ANALYZE, BUFFERS) 
SELECT * FROM users 
WHERE age > 25 
  AND city = 'New York' 
  AND category_id IN (1, 2, 3) 
ORDER BY embedding <=> '[1,2,3]' 
LIMIT 10;

-- Alternative test with different conditions
EXPLAIN (ANALYZE, BUFFERS)
SELECT id, age, city, embedding <=> '[2,3,4]' as distance 
FROM users 
WHERE age BETWEEN 25 AND 35
  AND city IN ('New York', 'Boston')
ORDER BY embedding <=> '[2,3,4]'
LIMIT 5;

-- Test to show bitmap scan usage in query plan
SET enable_seqscan = off;
SET enable_indexscan = off;
SET enable_bitmapscan = on;

EXPLAIN (ANALYZE, BUFFERS, VERBOSE)
SELECT * FROM users 
WHERE age > 20 
  AND city = 'New York'
  AND category_id = 1
ORDER BY embedding <=> '[1.5, 2.5, 3.5]'
LIMIT 3;
