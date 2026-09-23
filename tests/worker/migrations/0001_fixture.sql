CREATE TABLE users (
    id INTEGER PRIMARY KEY,
    email TEXT NOT NULL UNIQUE,
    name TEXT,
    age INTEGER CHECK (age >= 0),
    score REAL,
    active INTEGER NOT NULL DEFAULT 1,
    avatar BLOB
);

CREATE TABLE posts (
    id INTEGER PRIMARY KEY,
    user_id INTEGER NOT NULL REFERENCES users (id),
    title TEXT NOT NULL
);

CREATE TABLE numbers (
    id INTEGER PRIMARY KEY,
    value
);
