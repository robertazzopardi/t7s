-- d7s SQLite test database seed script
-- SQLite has no schemas/arrays/enums, so this mirrors the Postgres seed.sql
-- with a flattened, SQLite-compatible subset of tables and data.

CREATE TABLE users (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    username TEXT,
    email TEXT UNIQUE NOT NULL,
    full_name TEXT,
    age INTEGER,
    is_active BOOLEAN DEFAULT 1,
    created_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP,
    last_login TIMESTAMP,
    bio TEXT
);

INSERT INTO users (username, email, full_name, age, is_active, bio) VALUES
    ('john_doe', 'john@example.com', 'John Doe', 30, 1, 'Software developer'),
    ('jane_smith', 'jane@example.com', 'Jane Smith', 25, 1, 'Designer'),
    ('bob_wilson', 'bob@example.com', 'Bob Wilson', 35, 0, NULL),
    ('alice_brown', 'alice@example.com', 'Alice Brown', 28, 1, 'Data scientist'),
    ('charlie_davis', 'charlie@example.com', 'Charlie Davis', 42, 1, 'Project manager'),
    (NULL, 'no_username@example.com', 'No Username User', 33, 1, 'User without username'),
    ('grace_harris', 'grace@example.com', NULL, 26, 1, 'User with NULL full name');

CREATE TABLE products (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    name TEXT NOT NULL,
    description TEXT,
    price DECIMAL(10, 2) NOT NULL,
    quantity INTEGER DEFAULT 0,
    is_available BOOLEAN DEFAULT 1,
    created_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP,
    updated_at TIMESTAMP,
    category_id INTEGER REFERENCES users(id)
);

INSERT INTO products (name, description, price, quantity, is_available, category_id) VALUES
    ('Laptop', 'High-performance laptop', 1299.99, 50, 1, 1),
    ('Phone', 'Latest smartphone', 799.99, 100, 1, 1),
    ('Desk', 'Office desk', 299.99, 25, 1, NULL),
    ('Chair', 'Ergonomic office chair', 199.99, 30, 1, NULL),
    ('Keyboard', 'Mechanical keyboard', 89.99, 0, 0, 1);

-- Large table for virtual / paged table browsing (10k rows)
CREATE TABLE big_seed (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    payload TEXT NOT NULL,
    created_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP
);

WITH RECURSIVE seq(i) AS (
    SELECT 1
    UNION ALL
    SELECT i + 1 FROM seq WHERE i < 10000
)
INSERT INTO big_seed (payload)
SELECT 'row_' || i FROM seq;

-- Short | very long | short — open this and press W to see Fit vs Content.
CREATE TABLE wide_cols (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    code TEXT NOT NULL,
    payload TEXT NOT NULL,
    tag TEXT NOT NULL
);

INSERT INTO wide_cols (code, payload, tag) VALUES
    ('A1', 'lorem-ipsum-dolor-sit-amet-lorem-ipsum-dolor-sit-amet-lorem-ipsum-dolor-sit-amet-lorem-ipsum-dolor-sit-amet-lorem-ipsum-dolor-sit-amet-lorem-ipsum-dolor-sit-amet-', 'alpha'),
    ('B2', 'https://example.com/path/to/resource?id=aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa', 'beta'),
    ('C3', 'short-enough', 'gamma'),
    ('D4', 'WIDE_COLUMN_PAYLOAD_WIDE_COLUMN_PAYLOAD_WIDE_COLUMN_PAYLOAD_WIDE_COLUMN_PAYLOAD_WIDE_COLUMN_PAYLOAD_WIDE_COLUMN_PAYLOAD_WIDE_COLUMN_PAYLOAD_WIDE_COLUMN_PAYLOAD_', 'delta');

CREATE TABLE customers (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    name TEXT NOT NULL,
    email TEXT UNIQUE,
    phone TEXT,
    address TEXT,
    city TEXT,
    state TEXT,
    zip_code TEXT,
    created_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP
);

INSERT INTO customers (name, email, phone, address, city, state, zip_code) VALUES
    ('Customer One', 'customer1@example.com', '555-0100', '123 Main St', 'Springfield', 'IL', '62701'),
    ('Customer Two', 'customer2@example.com', '555-0101', '456 Oak Ave', 'Springfield', 'IL', '62702'),
    ('Customer Three', NULL, '555-0102', '789 Pine Rd', 'Shelbyville', 'IL', '62703');

CREATE TABLE orders (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    order_date TIMESTAMP DEFAULT CURRENT_TIMESTAMP,
    customer_id INTEGER REFERENCES customers(id),
    total_amount DECIMAL(10, 2),
    status TEXT DEFAULT 'pending',
    notes TEXT
);

INSERT INTO orders (order_date, customer_id, total_amount, status, notes) VALUES
    ('2024-01-15 10:30:00', 1, 1599.98, 'completed', 'Standard shipping'),
    ('2024-01-16 14:45:00', 2, 299.99, 'completed', NULL),
    (NULL, 1, 89.99, 'completed', 'Order with NULL date');

CREATE TABLE null_test (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    text_nullable TEXT,
    int_nullable INTEGER,
    bool_nullable BOOLEAN,
    timestamp_nullable TIMESTAMP,
    decimal_nullable DECIMAL(10, 2)
);

INSERT INTO null_test (text_nullable, int_nullable, bool_nullable, timestamp_nullable, decimal_nullable) VALUES
    ('Not NULL', 42, 1, '2024-01-15 10:30:00', 99.99),
    (NULL, NULL, NULL, NULL, NULL),
    ('Partial NULL', NULL, 1, NULL, 50.00);

CREATE TABLE special_chars (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    name_with_quotes TEXT,
    unicode_text TEXT,
    emoji_text TEXT,
    sql_injection_text TEXT
);

INSERT INTO special_chars (name_with_quotes, unicode_text, emoji_text, sql_injection_text) VALUES
    ('O''Brien', 'Hello 世界', 'Hello 👋🌍', 'SELECT * FROM users; DROP TABLE users;--'),
    ('Test "Quotes"', 'Привет мир', 'Test 😎', 'UNION SELECT * FROM passwords');

CREATE TABLE empty_table (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    name TEXT NOT NULL,
    description TEXT,
    created_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP
);

CREATE TABLE full_types (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    col_integer INTEGER,
    col_real REAL,
    col_text TEXT,
    col_boolean BOOLEAN,
    col_date DATE,
    col_timestamp TIMESTAMP,
    col_blob BLOB
);

INSERT INTO full_types (col_integer, col_real, col_text, col_boolean, col_date, col_timestamp, col_blob) VALUES
    (1000, 123.456789, 'This is text data', 1, '2024-01-15', '2024-01-15 10:30:00', NULL),
    (-1000, -123.456789, 'Negative numbers', 1, '2024-01-18', '2024-01-18 16:20:00', NULL),
    (NULL, NULL, NULL, NULL, NULL, NULL, NULL);
