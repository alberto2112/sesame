//! Sésame — le portail qui garde l'ordinateur.
//!
//! Le crate est une bibliothèque partagée par plusieurs binaires :
//!
//! - `sesame`       : le serveur. Un seul propriétaire de la base de données.
//! - `sesame-kiosk` : la porte d'entrée, avant que le bureau n'existe.
//!
//! Les binaires « système » (kiosque, minuteur, verrouillage) sont des clients
//! FINS : ils ne touchent pas la base, ils interrogent l'API du serveur. Une
//! seule source de vérité — [`policy::evaluate`] — et personne d'autre ne
//! décide.

pub mod admin;
pub mod auth;
pub mod config;
pub mod db;
pub mod importer;
pub mod policy;
pub mod quiz;
pub mod web;
