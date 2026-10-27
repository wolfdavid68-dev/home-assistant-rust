//! Tests de l'API : requêtes HTTP simulées, et un vrai serveur local pour le WebSocket.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    reason = "dans un test, un échec doit simplement faire échouer le test"
)]

// Concept Rust : `tests/api/main.rs` fait un seul programme de test
// (« api ») ; chaque `mod` ci-dessous est un fichier voisin. Un seul
// programme se compile plus vite que douze.
mod common;

mod assist;
mod auth;
mod automations;
mod dashboard;
mod mcp;
mod push;
mod reading;
mod relay;
mod roles;
mod services;
mod voice;
mod websocket;
