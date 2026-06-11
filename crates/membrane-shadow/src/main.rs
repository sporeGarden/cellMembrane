// SPDX-License-Identifier: AGPL-3.0-or-later

//! `membrane` CLI — sovereign shadow function dispatcher.
//!
//! Thin entry point: parses global flags, delegates to `dispatch::run`,
//! and formats output as human-readable or JSON.

use membrane_shadow::{ShadowConfig, ShadowOutcome};
use std::process::ExitCode;

fn usage() {
    eprintln!(
        r"membrane — sovereign shadow functions

Usage: membrane <domain.operation> [args...]

Repo (nestGate content.repo.*):
  repo.create <org/name>           Create repo on Forgejo
  repo.list <org>                  List repos in org
  repo.delete <org/name>           Delete repo from Forgejo

Mirror (nestGate content.mirror.*):
  mirror.sync <org/name>           Trigger mirror sync for one repo
  mirror.sync-all [org...]         Trigger sync on all mirrors (default: ecoPrimals)
  mirror.status <org/name>         Show mirror status for a repo
  mirror.push-create <org/name> <remote_url>  Create push mirror (Forgejo → GitHub)
  mirror.push-list <org/name>      List push mirrors for a repo
  mirror.push-sync <org/name>      Trigger push mirror sync

Service (biomeOS gate.service.*):
  service.list                     List running membrane services
  service.status <unit>            Show service status
  service.restart <unit>           Restart a service
  service.logs <unit> [lines]      Show recent logs (default: 30)

Gate (biomeOS gate.*):
  gate.info                        VPS system info + service summary
  gate.pull                        Run cascade-pull on VPS
  gate.check                       Parity check on VPS workspace
  gate.status                      Local gate health (JSON-RPC probe, depot, mesh)
  gate.profile <gate>              Show gate profile from ecosystem_manifest.toml
  gate.bootstrap <name> [--dry-run] [--mobile]
                                   One-command gate enrollment (fetch→verify→mesh→start→health)
                                   --mobile: write gate-name for NM reconnect hook

Temporal (waterFall temporal.*):
  temporal.check [repo_path...]    Temporal position matrix (local, all remotes)
  temporal.sync  [repo_path...]    Pull leader, push followers (ff-only)
  temporal.cascade [--gate auto] [--source temporal] [--check] [--clone-missing] [--no-freshness]
                   [--with-harvest] [--with-rebuild] [--check-installed]
                                   Full manifest-driven cascade sync (parallel, publishes freshness)
                                   --with-harvest: build drifted primals after sync, stage to depot
                                   --with-rebuild: harvest + refresh to VPS (full rebuild cycle)

Manifest (ecosystem manifest):
  manifest.info                    Show manifest metadata + sync config
  manifest.repos [gate]            List repos (all, or filtered by gate profile)
  manifest.orgs                    List all orgs from manifest

Identity:
  identity.resolve                 Show current gate identity

Impulse — rP action potentials (rootPulse ACTION):
  impulse.post --to <gate> --type <type> --subject <text>  Fire an impulse
  impulse.ack <id> [--note <text>]                         Acknowledge (receptor bind)
  impulse.archive                                          Discharge spent impulses

Potential — qS membrane potential (quorumSignal SENSE):
  potential.sense [--all] [--count]    Measure pending potential for this gate
  potential.check                      Gradient health across the mesh

Context — sweetGrass-external braids (developer state weaving):
  context.weave --project <path> --summary <text>  Weave a context braid
  context.sense [--gate <gate>] [--project <path>] [--all]  Sense context
  context.clear [--project <path>] [--expired]     Clear/decay braids

Plasmid (primal binary bootstrap):
  plasmid.fetch [--source github|vps|forgejo|wan] [--primal NAME] [--release TAG]
                [--force] [--dry-run] [--dest DIR]
                                   Fetch primal binaries with BLAKE3 verification
  plasmid.refresh [--primal NAME] [--source-dir DIR] [--dry-run]
                                   Push local binaries to VPS (atomic replace + restart)
  plasmid.harvest [--primal NAME] [--depot DIR] [--target TRIPLE] [--force] [--dry-run]
                                   Build from source, checksum, stage to depot
  plasmid.pipeline [--primal NAME] [--now] [--dry-run]
                                   Zero-touch: harvest → refresh → alive (full cycle)
  plasmid.trigger                  Remotely trigger VPS pipeline via SSH (immediate kick)
  plasmid.build <primal>            guideStone-grade single-primal build (ELF validation, provenance)
  plasmid.ndk.check                Check NDK toolchain for aarch64-linux-android cross-compile
  plasmid.depot_sync               Sync inner→outer membrane (BLAKE3 diff, atomic copy, checksums)
  plasmid.status                   Report depot freshness and upstream drift
  plasmid.staleness                Detect stale primals from provenance (local, no network)

Relay (K-Derm diderm relay chain):
  relay.run [repo_path...]         Full relay: pull → impulse → ship (metallic→ionic→weak)
  relay.mediate [repo_path...]     Pull from Forgejo only (metallic bond inward)
  relay.ship [repo_path...]        Push to GitHub via golgiBody-ext (ionic→weak outward)

Webhook (push-driven cascade):
  webhook.test <json_body>         Process a push event (dry-run: selective harvest)
  webhook.verify <body> --signature <hex>
                                   Verify HMAC-SHA256 signature (requires WEBHOOK_SECRET)

Caddy (outer membrane TLS / reverse proxy):
  caddy.status                     Service health and vhost count
  caddy.vhosts                     List configured vhosts
  caddy.tls.check <domain>         TLS certificate status
  caddy.tls.external <domain>      Switch domain to BearDog-managed certs (external files)
  caddy.tls.revert <domain>        Revert domain to Caddy-managed ACME
  caddy.reload                     Graceful reload
  caddy.validate                   Syntax check without apply
  caddy.depot.provision            Provision /depot/ file_server route (WAN binary distribution)
  caddy.depot.checksums            Serve checksums.toml from WAN endpoint
  caddy.acme.log [N]              Recent ACME/TLS certificate logs

Forgejo:
  forgejo.version                  Show Forgejo version

Options:
  --json                           Output as JSON (default: human-readable)
  -h, --help                       Show this help"
    );
}

#[tokio::main(flavor = "current_thread")]
async fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();

    let json_mode = args.iter().any(|a| a == "--json");
    let args: Vec<&str> = args
        .iter()
        .filter(|a| a.as_str() != "--json")
        .map(String::as_str)
        .collect();

    if args.is_empty() || args[0] == "-h" || args[0] == "help" {
        usage();
        return ExitCode::SUCCESS;
    }

    let config = ShadowConfig::from_env().await;
    let cmd = args[0];
    let rest = &args[1..];

    let outcome = membrane_shadow::dispatch::run(&config, cmd, rest).await;

    match outcome {
        Ok(ref o) => render(o, json_mode),
        Err(e) => {
            if json_mode {
                let o = ShadowOutcome::fail(&e);
                println!("{}", serde_json::to_string_pretty(&o).unwrap_or_default());
            } else {
                eprintln!("ERROR: {e}");
            }
            ExitCode::FAILURE
        }
    }
}

fn render(o: &ShadowOutcome, json_mode: bool) -> ExitCode {
    if json_mode {
        println!("{}", serde_json::to_string_pretty(&o).unwrap_or_default());
    } else {
        println!("{}", o.message);
        if let Some(data) = &o.data {
            println!("{}", serde_json::to_string_pretty(data).unwrap_or_default());
        }
    }
    if o.ok {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    }
}
