//! Sésame — le portail qui garde l'ordinateur.
//!
//! Le crate est une bibliothèque partagée par plusieurs binaires :
//!
//! - `sesame`       : le serveur. Un seul propriétaire de la base de données.
//! - `sesame-kiosk` : la porte d'entrée, dans `cage`, avant que le bureau
//!                    n'existe.
//! - `sesame-timer` : l'horloge. Elle décompte le temps accordé et ferme la
//!                    session quand il est épuisé — ce qui ramène à la porte.
//!
//! Les binaires « système » (kiosque, minuteur) sont des clients FINS : ils ne
//! touchent pas la base, ils interrogent l'API du serveur. Une seule source de
//! vérité — [`policy::evaluate`] — et personne d'autre ne décide.

pub mod admin;
pub mod auth;
pub mod config;
pub mod db;
pub mod dedup;
pub mod importer;
pub mod policy;
pub mod quiz;
pub mod web;
