-- d7s MySQL / MariaDB test database seed script (runs on both containers).
-- MySQL has no schemas below a database, so the Postgres schemas become
-- separate databases: d7s_test (core), shop, edge_cases.
-- Only syntax common to MySQL 8 and MariaDB 11 is used.

SET NAMES utf8mb4;

-- ---------------------------------------------------------------- d7s_test
USE d7s_test;

CREATE TABLE users (
    id INT AUTO_INCREMENT PRIMARY KEY,
    username VARCHAR(50),
    email VARCHAR(100) UNIQUE NOT NULL,
    full_name VARCHAR(100),
    age INT,
    is_active BOOLEAN DEFAULT TRUE,
    created_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP,
    last_login TIMESTAMP NULL,
    bio TEXT,
    KEY idx_users_username (username)
) DEFAULT CHARSET=utf8mb4;

INSERT INTO users (username, email, full_name, age, is_active, bio) VALUES
    ('john_doe', 'john@example.com', 'John Doe', 30, TRUE, 'Software developer'),
    ('jane_smith', 'jane@example.com', 'Jane Smith', 25, TRUE, 'Designer'),
    ('bob_wilson', 'bob@example.com', 'Bob Wilson', 35, FALSE, NULL),
    ('alice_brown', 'alice@example.com', 'Alice Brown', 28, TRUE, 'Data scientist'),
    ('charlie_davis', 'charlie@example.com', 'Charlie Davis', 42, TRUE, 'Project manager'),
    (NULL, 'no_username@example.com', 'No Username User', 33, TRUE, 'User without username'),
    ('grace_harris', 'grace@example.com', NULL, 26, TRUE, 'User with NULL full name');

CREATE TABLE products (
    id INT AUTO_INCREMENT PRIMARY KEY,
    name VARCHAR(100) NOT NULL,
    description TEXT,
    price DECIMAL(10, 2) NOT NULL,
    stock_quantity INT DEFAULT 0,
    category VARCHAR(50),
    created_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP,
    KEY idx_products_category (category)
) DEFAULT CHARSET=utf8mb4;

INSERT INTO products (name, description, price, stock_quantity, category) VALUES
    ('Laptop', 'High-performance laptop', 999.99, 50, 'Electronics'),
    ('Mouse', 'Wireless mouse', 29.99, 200, 'Electronics'),
    ('Keyboard', 'Mechanical keyboard', 79.99, 150, 'Electronics'),
    ('Desk Chair', 'Ergonomic office chair', 249.99, 75, 'Furniture'),
    ('Desk', 'Standing desk', 449.99, 30, 'Furniture'),
    ('Notebook', NULL, 4.99, 0, 'Stationery');

-- 10,000 rows to exercise paging / virtual scrolling.
CREATE TABLE big_seed (
    id INT AUTO_INCREMENT PRIMARY KEY,
    label VARCHAR(40) NOT NULL,
    value INT NOT NULL,
    created_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP
) DEFAULT CHARSET=utf8mb4;

-- Plain table: MySQL can't reference a TEMPORARY table more than once per query.
CREATE TABLE digits (d INT PRIMARY KEY);
INSERT INTO digits VALUES (0),(1),(2),(3),(4),(5),(6),(7),(8),(9);

INSERT INTO big_seed (label, value)
SELECT CONCAT('row-', n), (n * 7) % 1000
FROM (
    SELECT a.d + b.d * 10 + c.d * 100 + e.d * 1000 + 1 AS n
    FROM digits a, digits b, digits c, digits e
) t
ORDER BY n;

DROP TABLE digits;

-- -------------------------------------------------------------------- shop
CREATE DATABASE shop DEFAULT CHARACTER SET utf8mb4;
USE shop;

CREATE TABLE customers (
    id INT AUTO_INCREMENT PRIMARY KEY,
    name VARCHAR(100) NOT NULL,
    email VARCHAR(100) UNIQUE NOT NULL,
    country VARCHAR(50),
    created_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP
) DEFAULT CHARSET=utf8mb4;

INSERT INTO customers (name, email, country) VALUES
    ('Acme Corp', 'orders@acme.test', 'US'),
    ('Globex', 'buy@globex.test', 'DE'),
    ('Initech', 'it@initech.test', 'US'),
    ('Umbrella', 'supply@umbrella.test', NULL);

CREATE TABLE categories (
    id INT AUTO_INCREMENT PRIMARY KEY,
    name VARCHAR(50) NOT NULL UNIQUE
) DEFAULT CHARSET=utf8mb4;

INSERT INTO categories (name) VALUES ('Hardware'), ('Software'), ('Services');

CREATE TABLE orders (
    id INT AUTO_INCREMENT PRIMARY KEY,
    customer_id INT NOT NULL,
    status ENUM('pending', 'shipped', 'delivered', 'cancelled') NOT NULL DEFAULT 'pending',
    total DECIMAL(10, 2) NOT NULL,
    placed_at DATETIME DEFAULT CURRENT_TIMESTAMP,
    CONSTRAINT fk_orders_customer FOREIGN KEY (customer_id) REFERENCES customers (id),
    KEY idx_orders_status (status)
) DEFAULT CHARSET=utf8mb4;

INSERT INTO orders (customer_id, status, total, placed_at) VALUES
    (1, 'delivered', 1299.97, '2025-01-15 09:30:00'),
    (1, 'shipped', 79.99, '2025-02-02 14:05:00'),
    (2, 'pending', 249.99, '2025-02-10 11:00:00'),
    (3, 'cancelled', 29.99, '2025-02-11 16:45:00'),
    (4, 'pending', 449.99, '2025-02-12 08:20:00');

-- Composite primary key.
CREATE TABLE order_items (
    order_id INT NOT NULL,
    line_no INT NOT NULL,
    category_id INT,
    product VARCHAR(100) NOT NULL,
    quantity INT NOT NULL DEFAULT 1,
    unit_price DECIMAL(10, 2) NOT NULL,
    PRIMARY KEY (order_id, line_no),
    CONSTRAINT fk_items_order FOREIGN KEY (order_id) REFERENCES orders (id),
    CONSTRAINT fk_items_category FOREIGN KEY (category_id) REFERENCES categories (id)
) DEFAULT CHARSET=utf8mb4;

INSERT INTO order_items (order_id, line_no, category_id, product, quantity, unit_price) VALUES
    (1, 1, 1, 'Laptop', 1, 999.99),
    (1, 2, 1, 'Mouse', 2, 29.99),
    (1, 3, 1, 'Keyboard', 3, 79.99),
    (2, 1, 1, 'Keyboard', 1, 79.99),
    (3, 1, 1, 'Desk Chair', 1, 249.99),
    (4, 1, 1, 'Mouse', 1, 29.99),
    (5, 1, 1, 'Desk', 1, 449.99);

-- -------------------------------------------------------------- edge_cases
CREATE DATABASE edge_cases DEFAULT CHARACTER SET utf8mb4;
USE edge_cases;

CREATE TABLE null_test (
    id INT AUTO_INCREMENT PRIMARY KEY,
    nullable_text VARCHAR(50),
    nullable_int INT,
    nullable_date DATE,
    not_null_text VARCHAR(50) NOT NULL DEFAULT 'default'
) DEFAULT CHARSET=utf8mb4;

INSERT INTO null_test (nullable_text, nullable_int, nullable_date, not_null_text) VALUES
    ('has value', 1, '2025-01-01', 'a'),
    (NULL, NULL, NULL, 'b'),
    ('', 0, '2025-12-31', 'c');

CREATE TABLE empty_table (
    id INT AUTO_INCREMENT PRIMARY KEY,
    name VARCHAR(50)
) DEFAULT CHARSET=utf8mb4;

-- No primary key: shown read-only in d7s.
CREATE TABLE no_primary_key (
    name VARCHAR(50),
    score INT
) DEFAULT CHARSET=utf8mb4;

INSERT INTO no_primary_key VALUES ('alpha', 1), ('beta', 2), ('beta', 2);

CREATE TABLE special_chars (
    id INT AUTO_INCREMENT PRIMARY KEY,
    content VARCHAR(255)
) DEFAULT CHARSET=utf8mb4;

INSERT INTO special_chars (content) VALUES
    ('Single '' quote'),
    ('Double " quote'),
    ('Back`tick'),
    ('Backslash \\ here'),
    ('Tab	and newline
second line'),
    ('Unicode: héllo wörld ñ 日本語 🚀'),
    ('<script>alert("xss")</script>'),
    ('; DROP TABLE users; --');

-- Table and column names that need quoting.
CREATE TABLE `weird name` (
    `select` INT PRIMARY KEY,
    `col``tick` VARCHAR(20)
) DEFAULT CHARSET=utf8mb4;

INSERT INTO `weird name` VALUES (1, 'one'), (2, 'two');

CREATE TABLE long_text (
    id INT AUTO_INCREMENT PRIMARY KEY,
    short_text VARCHAR(20),
    long_text TEXT
) DEFAULT CHARSET=utf8mb4;

INSERT INTO long_text (short_text, long_text) VALUES
    ('short', REPEAT('The quick brown fox jumps over the lazy dog. ', 40)),
    ('empty', '');

-- Short | very long | short — open this and press W to see Fit vs Content.
CREATE TABLE wide_cols (
    id INT AUTO_INCREMENT PRIMARY KEY,
    code VARCHAR(8) NOT NULL,
    payload TEXT NOT NULL,
    tag VARCHAR(8) NOT NULL
) DEFAULT CHARSET=utf8mb4;

INSERT INTO wide_cols (code, payload, tag) VALUES
    ('A1', REPEAT('lorem-ipsum-dolor-sit-amet-', 12), 'alpha'),
    ('B2', CONCAT(REPEAT('https://example.com/path/to/resource?id=', 8), 'zzzz'), 'beta'),
    ('C3', 'short-enough', 'gamma'),
    ('D4', REPEAT('WIDE_COLUMN_PAYLOAD_', 20), 'delta');

-- Wide variety of column types, including binary / bit / json / date-time.
CREATE TABLE full_types (
    id INT AUTO_INCREMENT PRIMARY KEY,
    c_tinyint TINYINT,
    c_bigint BIGINT,
    c_unsigned INT UNSIGNED,
    c_decimal DECIMAL(12, 4),
    c_float FLOAT,
    c_double DOUBLE,
    c_bool BOOLEAN,
    c_char CHAR(5),
    c_date DATE,
    c_time TIME,
    c_datetime DATETIME,
    c_timestamp TIMESTAMP NULL,
    c_year YEAR,
    c_enum ENUM('red', 'green', 'blue'),
    c_set SET('a', 'b', 'c'),
    c_json JSON,
    c_blob BLOB,
    c_varbinary VARBINARY(16),
    c_bit BIT(8)
) DEFAULT CHARSET=utf8mb4;

INSERT INTO full_types (
    c_tinyint, c_bigint, c_unsigned, c_decimal, c_float, c_double, c_bool, c_char,
    c_date, c_time, c_datetime, c_timestamp, c_year, c_enum, c_set, c_json,
    c_blob, c_varbinary, c_bit
) VALUES
    (127, 9223372036854775807, 4294967295, 12345.6789, 3.14, 2.718281828, TRUE, 'abc',
     '2025-06-15', '13:45:30', '2025-06-15 13:45:30', '2025-06-15 13:45:30', 2025, 'green', 'a,c',
     '{"name": "d7s", "tags": ["tui", "db"], "nested": {"ok": true}}',
     X'DEADBEEF', X'CAFEBABE', b'10101010'),
    (NULL, NULL, NULL, NULL, NULL, NULL, NULL, NULL,
     NULL, NULL, NULL, NULL, NULL, NULL, NULL, NULL,
     NULL, NULL, NULL);
