// SPDX-License-Identifier: AGPL-3.0-or-later

use cellmembrane_types::composition::MembraneComposition;
use cellmembrane_types::service::MembraneService;

// --- Service registry ---

#[test]
fn service_lookup_all_primals() {
    for name in [
        "beardog",
        "songbird",
        "skunkbat",
        "nestgate",
        "rhizocrypt",
        "loamspine",
        "sweetgrass",
    ] {
        assert!(
            MembraneService::for_binary(name).is_some(),
            "Service not found for primal: {name}"
        );
    }
}

#[test]
fn service_lookup_symbiotic() {
    for name in ["hbbs", "hbbr", "caddy"] {
        let svc = MembraneService::for_binary(name)
            .unwrap_or_else(|| panic!("Service not found: {name}"));
        assert!(!svc.is_primal, "{name} should not be marked as primal");
    }
}

#[test]
fn beardog_is_uds_only() {
    let svc = MembraneService::for_binary("beardog").unwrap();
    assert!(svc.has_socket);
    assert!(svc.port.is_none());
    assert!(!svc.is_externally_reachable());
}

#[test]
fn skunkbat_is_loopback_only() {
    let svc = MembraneService::for_binary("skunkbat").unwrap();
    assert_eq!(svc.bind, "127.0.0.1");
    assert!(!svc.is_externally_reachable());
}

#[test]
fn songbird_is_externally_reachable() {
    let svc = MembraneService::for_binary("songbird").unwrap();
    assert!(svc.is_externally_reachable());
    assert_eq!(svc.port, Some(3478));
}

#[test]
fn static_registry_returns_references() {
    let a = MembraneService::for_binary("beardog").unwrap();
    let b = MembraneService::for_binary("beardog").unwrap();
    assert!(
        std::ptr::eq(a, b),
        "Static registry should return same reference"
    );
}

// --- Binary integrity ---

#[test]
fn binary_integrity_relay_has_songbird() {
    let bins = cellmembrane_types::service::binary_integrity_for(MembraneComposition::Relay);
    assert!(
        bins.iter().any(|b| b.binary == "songbird"),
        "Relay must verify songbird binary"
    );
    let songbird = bins.iter().find(|b| b.binary == "songbird").unwrap();
    assert_eq!(
        songbird.hash_algorithm,
        cellmembrane_types::service::HashAlgorithm::Blake3
    );
    assert!(songbird.require_static);
}

#[test]
fn binary_integrity_tower_has_all_primals() {
    let bins = cellmembrane_types::service::binary_integrity_for(MembraneComposition::Tower);
    for primal in ["beardog", "songbird", "skunkbat"] {
        assert!(
            bins.iter().any(|b| b.binary == primal),
            "Tower must verify {primal}"
        );
    }
}

#[test]
fn binary_integrity_symbiotic_use_sha256() {
    let bins = cellmembrane_types::service::binary_integrity_for(MembraneComposition::Tower);
    for sym in ["hbbs", "hbbr"] {
        let entry = bins.iter().find(|b| b.binary == sym);
        assert!(entry.is_some(), "Tower must verify {sym}");
        assert_eq!(
            entry.unwrap().hash_algorithm,
            cellmembrane_types::service::HashAlgorithm::Sha256,
            "Symbiotic {sym} should use SHA-256"
        );
        assert!(!entry.unwrap().require_static);
    }
}

#[test]
fn binary_integrity_grows_with_composition() {
    let relay = cellmembrane_types::service::binary_integrity_for(MembraneComposition::Relay);
    let tower = cellmembrane_types::service::binary_integrity_for(MembraneComposition::Tower);
    let nest = cellmembrane_types::service::binary_integrity_for(MembraneComposition::Nest);
    assert!(tower.len() > relay.len());
    assert!(nest.len() > tower.len());
}

#[test]
fn binary_integrity_install_path_matches_service_registry() {
    let paths = cellmembrane_types::service::ServicePaths::from_env();
    for comp in MembraneComposition::all() {
        let bins = cellmembrane_types::service::binary_integrity_for(*comp);
        for entry in &bins {
            let svc = MembraneService::for_binary(entry.binary)
                .unwrap_or_else(|| panic!("No service for {}", entry.binary));
            assert_eq!(
                entry.install_path,
                paths.install_path(svc),
                "BinaryIntegrity path should match service registry for {}",
                entry.binary,
            );
        }
    }
}

// --- Credential files ---

#[test]
fn relay_credential_files_include_turn() {
    let files = cellmembrane_types::credentials::credential_files_for(MembraneComposition::Relay);
    assert!(
        files
            .iter()
            .any(|f| f.path.contains("songbird") || f.path.contains("relay-credentials")),
        "Relay must have TURN credential files"
    );
    for f in &files {
        assert_eq!(f.expected_owner, "root");
    }
}

#[test]
fn rustdesk_credential_files_include_key() {
    let files =
        cellmembrane_types::credentials::credential_files_for(MembraneComposition::RustDesk);
    assert!(
        files.iter().any(|f| f.path.contains("id_ed25519.pub")),
        "RustDesk must have public key file"
    );
    assert!(
        files
            .iter()
            .any(|f| f.path.contains("id_ed25519") && !f.path.contains(".pub")),
        "RustDesk must have private key file"
    );
}

#[test]
fn tower_credential_files_include_tower_env() {
    let files = cellmembrane_types::credentials::credential_files_for(MembraneComposition::Tower);
    let tower_env = files.iter().find(|f| f.path.contains("tower.env"));
    assert!(tower_env.is_some(), "Tower must have tower.env");
    assert_eq!(tower_env.unwrap().expected_mode, "600");
}

#[test]
fn credential_files_grow_with_composition() {
    let relay = cellmembrane_types::credentials::credential_files_for(MembraneComposition::Relay);
    let rustdesk =
        cellmembrane_types::credentials::credential_files_for(MembraneComposition::RustDesk);
    let tower = cellmembrane_types::credentials::credential_files_for(MembraneComposition::Tower);
    assert!(
        rustdesk.len() > relay.len(),
        "RustDesk should have more credential files than Relay"
    );
    assert!(
        tower.len() > rustdesk.len(),
        "Tower should have more credential files than RustDesk"
    );
}
