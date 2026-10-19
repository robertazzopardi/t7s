-- d7s Test Database Seed Script
-- This script creates comprehensive test data for testing the d7s database explorer app

-- Schema: public - default schema with standard tables
CREATE TABLE public.users (
    id SERIAL PRIMARY KEY,
    username VARCHAR(50),
    email VARCHAR(100) UNIQUE NOT NULL,
    full_name VARCHAR(100),
    age INTEGER,
    is_active BOOLEAN DEFAULT true,
    created_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP,
    last_login TIMESTAMP,
    bio TEXT
);

CREATE TABLE public.products (
    id SERIAL PRIMARY KEY,
    name VARCHAR(100) NOT NULL,
    description TEXT,
    price DECIMAL(10, 2) NOT NULL,
    quantity INTEGER DEFAULT 0,
    is_available BOOLEAN DEFAULT true,
    created_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP,
    updated_at TIMESTAMP,
    category_id INTEGER REFERENCES public.users(id),
    tags TEXT[]
);

INSERT INTO public.users (username, email, full_name, age, is_active, bio) VALUES
    ('john_doe', 'john@example.com', 'John Doe', 30, true, 'Software developer'),
    ('jane_smith', 'jane@example.com', 'Jane Smith', 25, true, 'Designer'),
    ('bob_wilson', 'bob@example.com', 'Bob Wilson', 35, false, NULL),
    ('alice_brown', 'alice@example.com', 'Alice Brown', 28, true, 'Data scientist'),
    ('charlie_davis', 'charlie@example.com', 'Charlie Davis', 42, true, 'Project manager'),
    ('diana_evans', 'diana@example.com', 'Diana Evans', 31, true, 'DevOps engineer'),
    ('edward_foster', 'edward@example.com', 'Edward Foster', 29, false, 'QA tester'),
    (NULL, 'no_username@example.com', 'No Username User', 33, true, 'User without username'),
    ('frank_garcia', 'frank.garcia@example.com', 'Frank Garcia', 27, true, NULL),
    ('grace_harris', 'grace@example.com', NULL, 26, true, 'User with NULL full name');

-- Large table for virtual / paged table browsing (10k rows)
CREATE TABLE public.big_seed (
    id SERIAL PRIMARY KEY,
    payload TEXT NOT NULL,
    created_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP
);

INSERT INTO public.big_seed (payload)
SELECT 'row_' || gs.i::text
FROM generate_series(1, 10000) AS gs(i);

-- Short | very long | short — open this and press W to see Fit vs Content.
CREATE TABLE public.wide_cols (
    id SERIAL PRIMARY KEY,
    code VARCHAR(8) NOT NULL,
    payload TEXT NOT NULL,
    tag VARCHAR(8) NOT NULL
);

INSERT INTO public.wide_cols (code, payload, tag) VALUES
    ('A1', repeat('lorem-ipsum-dolor-sit-amet-', 12), 'alpha'),
    ('B2', repeat('https://example.com/path/to/resource?id=', 8) || 'zzzz', 'beta'),
    ('C3', 'short-enough', 'gamma'),
    ('D4', repeat('WIDE_COLUMN_PAYLOAD_', 20), 'delta');

COMMENT ON TABLE public.wide_cols IS 'Column-width demo: fat middle column; toggle with W';

INSERT INTO public.products (name, description, price, quantity, is_available, category_id, tags) VALUES
    ('Laptop', 'High-performance laptop', 1299.99, 50, true, 1, ARRAY['electronics', 'computers']),
    ('Phone', 'Latest smartphone', 799.99, 100, true, 1, ARRAY['electronics', 'mobile']),
    ('Desk', 'Office desk', 299.99, 25, true, NULL, ARRAY['furniture']),
    ('Chair', 'Ergonomic office chair', 199.99, 30, true, NULL, ARRAY['furniture']),
    ('Monitor', '27-inch monitor', 349.99, 40, true, 1, ARRAY['electronics']),
    ('Product with no name', 'Product with minimal details', 9.99, 5, true, NULL, ARRAY['misc']),
    ('Keyboard', 'Mechanical keyboard', 89.99, 0, false, 1, ARRAY['electronics', 'accessories']),
    ('Mouse', 'Wireless mouse', 29.99, 60, true, 1, ARRAY['electronics', 'accessories']),
    ('Headphones', 'Noise-canceling headphones', 149.99, 20, true, 2, ARRAY['electronics', 'audio']),
    ('Lamp', 'LED desk lamp', 49.99, 15, true, NULL, ARRAY['furniture', 'lighting']);

-- Schema: test_schema - basic test tables with relationships
CREATE SCHEMA test_schema;

CREATE TABLE test_schema.customers (
    id SERIAL PRIMARY KEY,
    name VARCHAR(100) NOT NULL,
    email VARCHAR(100) UNIQUE,
    phone VARCHAR(20),
    address TEXT,
    city VARCHAR(50),
    state VARCHAR(50),
    zip_code VARCHAR(10),
    created_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP
);

CREATE TABLE test_schema.categories (
    id SERIAL PRIMARY KEY,
    name VARCHAR(50) NOT NULL UNIQUE,
    description TEXT,
    parent_category_id INTEGER REFERENCES test_schema.categories(id)
);

CREATE TABLE test_schema.orders (
    id SERIAL PRIMARY KEY,
    order_date TIMESTAMP DEFAULT CURRENT_TIMESTAMP,
    customer_id INTEGER REFERENCES test_schema.customers(id),
    total_amount DECIMAL(10, 2),
    status VARCHAR(20) DEFAULT 'pending',
    notes TEXT
);

CREATE TABLE test_schema.order_items (
    id SERIAL PRIMARY KEY,
    order_id INTEGER REFERENCES test_schema.orders(id),
    product_name VARCHAR(100),
    quantity INTEGER NOT NULL,
    unit_price DECIMAL(10, 2),
    discount DECIMAL(5, 2)
);

INSERT INTO test_schema.customers (name, email, phone, address, city, state, zip_code) VALUES
    ('Customer One', 'customer1@example.com', '555-0100', '123 Main St', 'Springfield', 'IL', '62701'),
    ('Customer Two', 'customer2@example.com', '555-0101', '456 Oak Ave', 'Springfield', 'IL', '62702'),
    ('Customer Three', NULL, '555-0102', '789 Pine Rd', 'Shelbyville', 'IL', '62703'),
    ('Customer Four', 'customer4@example.com', NULL, '321 Elm Blvd', 'Capital City', 'IL', '62704'),
    ('Customer Five', 'customer5@example.com', '555-0104', '654 Maple Dr', 'Ogdenville', 'IL', '62705');

INSERT INTO test_schema.categories (name, description, parent_category_id) VALUES
    ('Electronics', 'Electronic devices', NULL),
    ('Computers', 'Computer equipment', 1),
    ('Mobile', 'Mobile devices', 1),
    ('Furniture', 'Office furniture', NULL),
    ('Lighting', 'Lighting products', 4),
    ('Accessories', 'Accessories and peripherals', 1);

INSERT INTO test_schema.orders (order_date, customer_id, total_amount, status, notes) VALUES
    ('2024-01-15 10:30:00', 1, 1599.98, 'completed', 'Standard shipping'),
    ('2024-01-16 14:45:00', 2, 299.99, 'completed', NULL),
    ('2024-01-17 09:00:00', 3, 979.98, 'shipped', 'Express delivery'),
    ('2024-01-18 16:20:00', 4, 549.97, 'pending', 'Gift wrapping requested'),
    ('2024-01-19 11:10:00', 5, 199.99, 'completed', NULL),
    (NULL, 1, 89.99, 'completed', 'Order with NULL date'),
    ('2024-01-20 12:00:00', 2, 239.98, 'processing', 'Second order for customer');

INSERT INTO test_schema.order_items (order_id, product_name, quantity, unit_price, discount) VALUES
    (1, 'Laptop', 1, 1299.99, 0.00),
    (1, 'Phone', 1, 799.99, 0.00),
    (2, 'Desk', 1, 299.99, 0.00),
    (3, 'Monitor', 2, 349.99, 0.10),
    (3, 'Keyboard', 1, 89.99, 0.00),
    (4, 'Chair', 2, 199.99, 0.05),
    (4, 'Lamp', 1, 49.99, 0.00),
    (5, 'Mouse', 1, 29.99, 0.00),
    (5, 'Headphones', 1, 149.99, 0.20),
    (6, 'Keyboard', 1, 89.99, 0.00);

-- Schema: edge_cases - tables for testing edge cases
CREATE SCHEMA edge_cases;

CREATE TABLE edge_cases.null_test (
    id SERIAL PRIMARY KEY,
    text_nullable VARCHAR(50),
    int_nullable INTEGER,
    bool_nullable BOOLEAN,
    timestamp_nullable TIMESTAMP,
    decimal_nullable DECIMAL(10, 2),
    text_array_nullable TEXT[]
);

CREATE TABLE edge_cases.empty_table (
    id SERIAL PRIMARY KEY,
    name VARCHAR(100) NOT NULL,
    description TEXT,
    created_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP
);

CREATE TABLE edge_cases.special_chars (
    id SERIAL PRIMARY KEY,
    name_with_quotes VARCHAR(100),
    name_with_newlines TEXT,
    unicode_text TEXT,
    emoji_text TEXT,
    sql_injection_text TEXT
);

CREATE TABLE edge_cases.long_text (
    id SERIAL PRIMARY KEY,
    title VARCHAR(100),
    content TEXT,
    metadata TEXT
);

INSERT INTO edge_cases.null_test (text_nullable, int_nullable, bool_nullable, timestamp_nullable, decimal_nullable, text_array_nullable) VALUES
    ('Not NULL', 42, true, '2024-01-15 10:30:00', 99.99, ARRAY['a', 'b', 'c']),
    (NULL, NULL, NULL, NULL, NULL, NULL),
    ('Partial NULL', NULL, true, NULL, 50.00, ARRAY['x', 'y']),
    ('Another value', 100, false, '2024-01-16 14:45:00', NULL, NULL),
    (NULL, 200, NULL, '2024-01-17 09:00:00', 75.50, ARRAY['1', '2', '3']),
    ('All NULL except text', 0, false, NULL, 0.00, NULL),
    ('Text with NULL others', NULL, true, '2024-01-18 16:20:00', NULL, NULL),
    (NULL, NULL, NULL, '2024-01-19 11:10:00', NULL, NULL);

INSERT INTO edge_cases.special_chars (name_with_quotes, unicode_text, emoji_text, sql_injection_text) VALUES
    ('O''Brien', 'Hello 世界', 'Hello 👋🌍', 'SELECT * FROM users; DROP TABLE users;--'),
    ('Test "Quotes"', 'Привет мир', 'Test 😎', 'UNION SELECT * FROM passwords'),
    ('Back\\slash', 'مرحبا', '🚀🎉', '; DROP TABLE customers; --'),
    ('Apostrophe''s test', 'こんにちは', '💻🔥', ''' OR ''1''=''1'),
    ('Multiple """" quotes', '한글', '🎨🎭', '1; DROP DATABASE d7s_test;--');

-- Helper function to repeat strings
CREATE OR REPLACE FUNCTION repeat_string(TEXT, INTEGER) RETURNS TEXT AS $$
BEGIN
    RETURN repeat($1, $2);
END;
$$ LANGUAGE plpgsql;

INSERT INTO edge_cases.long_text (title, content, metadata) VALUES
    ('Short post', 'This is a short post.', '{"length": "short"}'),
    ('Medium post', 'This is a medium-length post with some more content but still not too long. It has multiple sentences and provides a decent amount of text.', '{"length": "medium"}'),
    ('Long post', repeat_string('This is a very long post. ', 50), '{"length": "long", "estimated_words": 300}'),
    -- Mid-column overflow with short neighbors (same exercise as public.wide_cols).
    ('Wide mid', repeat_string('column-overflow-demo-', 15), 'tail');

-- Schema: complex_types - tables with advanced Postgres data types
CREATE SCHEMA complex_types;

CREATE TABLE complex_types.json_data (
    id SERIAL PRIMARY KEY,
    metadata JSON,
    config JSONB,
    mixed_data JSON,
    data_version INTEGER DEFAULT 1
);

CREATE TABLE complex_types.arrays (
    id SERIAL PRIMARY KEY,
    text_array TEXT[],
    int_array INTEGER[],
    decimal_array DECIMAL(10, 2)[],
    bool_array BOOLEAN[],
    mixed_array TEXT[]
);

CREATE TABLE complex_types.uuids (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    reference_uuid UUID,
    name VARCHAR(100),
    created_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP
);

CREATE TABLE complex_types.binary_data (
    id SERIAL PRIMARY KEY,
    file_name VARCHAR(100),
    file_content BYTEA,
    file_size INTEGER,
    file_hash VARCHAR(64),
    uploaded_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP
);

CREATE TABLE complex_types.full_types (
    id SERIAL PRIMARY KEY,
    col_smallint SMALLINT,
    col_integer INTEGER,
    col_bigint BIGINT,
    col_decimal DECIMAL(10, 4),
    col_real REAL,
    col_double DOUBLE PRECISION,
    col_varchar VARCHAR(50),
    col_char CHAR(10),
    col_text TEXT,
    col_boolean BOOLEAN,
    col_date DATE,
    col_time TIME,
    col_timestamp TIMESTAMP,
    col_timestamptz TIMESTAMPTZ,
    col_uuid UUID,
    col_json JSON,
    col_jsonb JSONB,
    col_bytea BYTEA,
    col_array_int INTEGER[],
    col_array_text TEXT[],
    col_array_decimal DECIMAL(10, 2)[]
);

INSERT INTO complex_types.json_data (metadata, config, mixed_data) VALUES
    ('{"name": "test1", "value": 100}', '{"active": true, "settings": {"theme": "dark"}}', '{"string": "text", "number": 42, "bool": true}'),
    ('{"name": "test2", "value": 200}', '{"active": false, "settings": {"theme": "light"}}', '{"nested": {"deep": {"value": 123}}}'),
    ('{"array": [1,2,3], "obj": {"x": 1}}', '{"features": ["feature1", "feature2"]}', 'null'),
    (NULL, '{}', '{"empty": {}}'),
    ('{"complex": {"nested": {"deep": "value"}}}', '{"items": [{"id": 1}, {"id": 2}]}', '{"bool": false, "num": 0, "str": ""}');

INSERT INTO complex_types.arrays (text_array, int_array, decimal_array, bool_array, mixed_array) VALUES
    (ARRAY['apple', 'banana', 'cherry'], ARRAY[1, 2, 3, 4, 5], ARRAY[1.99, 2.99, 3.99], ARRAY[true, false, true], ARRAY['mixed', '123', 'text', 'data']),
    (ARRAY['one', 'two'], ARRAY[10, 20, 30], ARRAY[10.50, 20.50], ARRAY[false, false], ARRAY['a', 'b']),
    (ARRAY[]::TEXT[], ARRAY[]::INTEGER[], ARRAY[]::DECIMAL(10, 2)[], ARRAY[]::BOOLEAN[], NULL),
    (ARRAY['single'], ARRAY[100], ARRAY[99.99], ARRAY[true], ARRAY['only', 'one']),
    (ARRAY['x', 'y', 'z', 'w'], ARRAY[1, 2, 3, 4, 5, 6], ARRAY[0.10, 0.20, 0.30, 0.40], ARRAY[true, true, false, true, false], ARRAY['alpha', 'beta', 'gamma', 'delta', 'epsilon']);

INSERT INTO complex_types.uuids (reference_uuid, name) VALUES
    (gen_random_uuid(), 'Item 1'),
    (gen_random_uuid(), 'Item 2'),
    (NULL, 'Item 3'),
    ('123e4567-e89b-12d3-a456-426614174000', 'Item 4'),
    (gen_random_uuid(), NULL);

INSERT INTO complex_types.binary_data (file_name, file_size, file_hash) VALUES
    ('test1.txt', 1024, '5d41402abc4b2a76b9719d911017c592'),
    ('test2.bin', 2048, 'e99a18c428cb38d5f260853678922e03'),
    ('test3.dat', 512, NULL),
    ('test4.jpg', 4096, 'a87ff679a2f3e71d9181a67b7542122c'),
    (NULL, 0, 'd41d8cd98f00b204e9800998ecf8427e');

INSERT INTO complex_types.full_types (
    col_smallint, col_integer, col_bigint, col_decimal, col_real, col_double,
    col_varchar, col_char, col_text, col_boolean, col_date, col_time, col_timestamp,
    col_timestamptz, col_uuid, col_json, col_jsonb,
    col_array_int, col_array_text, col_array_decimal
) VALUES
    (100, 1000, 1000000, 123.4567, 123.456, 123.456789,
     'test', 'fixed', 'This is text data', true, '2024-01-15', '10:30:00', '2024-01-15 10:30:00',
     '2024-01-15 10:30:00+00', gen_random_uuid(), '{"key": "value"}', '{"key": "value"}',
     ARRAY[1, 2, 3], ARRAY['a', 'b', 'c'], ARRAY[1.1, 2.2, 3.3]),
    (200, 2000, 2000000, 789.0123, 789.012, 789.012345,
     'another', 'another', 'More text', false, '2024-01-16', '14:45:00', '2024-01-16 14:45:00',
     '2024-01-16 14:45:00+00', gen_random_uuid(), '{"nested": {"value": 123}}', '{"active": true}',
     ARRAY[10, 20], ARRAY['x', 'y'], ARRAY[10.5, 20.5]),
    (NULL, NULL, NULL, NULL, NULL, NULL,
     NULL, NULL, NULL, NULL, NULL, NULL, NULL,
     NULL, NULL, NULL, NULL,
     ARRAY[]::INTEGER[], ARRAY[]::TEXT[], ARRAY[]::DECIMAL(10, 2)[]),
    (-100, -1000, -1000000, -123.4567, -123.456, -123.456789,
     'negative', 'negative', 'Negative numbers', true, '2024-01-18', '16:20:00', '2024-01-18 16:20:00',
     '2024-01-18 16:20:00+00', gen_random_uuid(), '{"negative": -100}', '{"neg": true}',
     ARRAY[-1, -2, -3], ARRAY['neg'], ARRAY[-1.1, -2.2]),
    (0, 0, 0, 0.0, 0.0, 0.0,
     'zero', '0', 'Zero values', false, '2024-01-19', '11:10:00', '2024-01-19 11:10:00',
     '2024-01-19 11:10:00+00', '123e4567-e89b-12d3-a456-426614174000', '{"zero": 0}', '{"is_zero": false}',
     ARRAY[0], ARRAY['zero'], ARRAY[0.0]);

-- Custom enum types for the complex_types schema
CREATE TYPE complex_types.user_role AS ENUM ('admin', 'moderator', 'editor', 'viewer', 'guest');
CREATE TYPE complex_types.order_status AS ENUM ('pending', 'processing', 'shipped', 'delivered', 'cancelled', 'refunded');
CREATE TYPE complex_types.priority_level AS ENUM ('critical', 'high', 'medium', 'low', 'none');
CREATE TYPE complex_types.day_of_week AS ENUM ('monday', 'tuesday', 'wednesday', 'thursday', 'friday', 'saturday', 'sunday');
CREATE TYPE complex_types.color AS ENUM ('red', 'orange', 'yellow', 'green', 'blue', 'indigo', 'violet');

CREATE TABLE complex_types.enum_types (
    id SERIAL PRIMARY KEY,
    label VARCHAR(100),
    role complex_types.user_role,
    status complex_types.order_status,
    priority complex_types.priority_level,
    scheduled_day complex_types.day_of_week,
    badge_color complex_types.color,
    created_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP
);

INSERT INTO complex_types.enum_types (label, role, status, priority, scheduled_day, badge_color) VALUES
    ('Alice Admin',       'admin',     'delivered',  'critical', 'monday',    'red'),
    ('Bob Moderator',     'moderator', 'shipped',     'high',    'wednesday', 'orange'),
    ('Carol Editor',      'editor',    'processing',  'medium',  'friday',    'yellow'),
    ('Dave Viewer',       'viewer',    'pending',     'low',     'tuesday',   'green'),
    ('Eve Guest',         'guest',     'cancelled',   'none',    'saturday',  'blue'),
    ('Frank Admin',       'admin',     'refunded',    'high',    'thursday',  'indigo'),
    ('Grace Moderator',   'moderator', 'delivered',   'medium',  'sunday',    'violet'),
    ('Hank Editor',       'editor',    'shipped',     'critical','monday',    'red'),
    ('Ivy Viewer',        'viewer',    'processing',  'low',     'wednesday', 'green'),
    ('Jack Guest',        'guest',     'pending',     'none',    'friday',    'blue'),
    ('Kate Admin',        'admin',     'delivered',   'high',    'tuesday',   'orange'),
    ('Liam Editor',       'editor',    'cancelled',   'medium',  'saturday',  'yellow'),
    -- NULLs to test nullable enum columns
    ('Null Role',         NULL,        'pending',     'low',     'monday',    'red'),
    ('Null Status',       'viewer',    NULL,          'none',    'thursday',  'violet'),
    ('All Nulls',         NULL,        NULL,          NULL,      NULL,        NULL);

-- Add comments for better documentation
COMMENT ON SCHEMA public IS 'Default schema for standard tables';
COMMENT ON SCHEMA test_schema IS 'Schema with basic test tables and relationships';
COMMENT ON SCHEMA edge_cases IS 'Schema with edge case scenarios and special data';
COMMENT ON SCHEMA complex_types IS 'Schema with advanced Postgres data types';

-- Grant necessary permissions
GRANT ALL PRIVILEGES ON ALL TABLES IN SCHEMA public TO d7s_user;
GRANT ALL PRIVILEGES ON ALL TABLES IN SCHEMA test_schema TO d7s_user;
GRANT ALL PRIVILEGES ON ALL TABLES IN SCHEMA edge_cases TO d7s_user;
GRANT ALL PRIVILEGES ON ALL TABLES IN SCHEMA complex_types TO d7s_user;

GRANT ALL PRIVILEGES ON ALL SEQUENCES IN SCHEMA public TO d7s_user;
GRANT ALL PRIVILEGES ON ALL SEQUENCES IN SCHEMA test_schema TO d7s_user;
GRANT ALL PRIVILEGES ON ALL SEQUENCES IN SCHEMA edge_cases TO d7s_user;
GRANT ALL PRIVILEGES ON ALL SEQUENCES IN SCHEMA complex_types TO d7s_user;
GRANT USAGE ON TYPE complex_types.user_role TO d7s_user;
GRANT USAGE ON TYPE complex_types.order_status TO d7s_user;
GRANT USAGE ON TYPE complex_types.priority_level TO d7s_user;
GRANT USAGE ON TYPE complex_types.day_of_week TO d7s_user;
GRANT USAGE ON TYPE complex_types.color TO d7s_user;
