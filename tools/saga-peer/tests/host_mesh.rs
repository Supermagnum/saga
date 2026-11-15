//! Host CI: full-mesh encrypted calls (3 and 4 peers), leave, and the cap.

use saga_peer::mesh::{
    host_mesh_roundtrip, HostMesh, MeshFullError, MixCheck, MESH_DEFAULT_FRAMES,
};
use saga_peer::{LABEL_ALICE, LABEL_BOB, LABEL_CHARLENE, LABEL_LAPTOP};

const PRIME: usize = 10;

fn assert_outcome(labels: &[&str], encrypted: bool, out: &saga_peer::MeshOutcome) {
    let n = labels.len();
    assert_eq!(out.links, n * (n - 1) / 2);
    if encrypted {
        // One key per link, all distinct (checked at setup too).
        assert_eq!(out.link_keys.len(), out.links);
        for (i, a) in out.link_keys.iter().enumerate() {
            for b in &out.link_keys[i + 1..] {
                assert_ne!(a.key, b.key, "link keys must differ across links");
            }
        }
        // Both ends of every link receive and verify a roster.
        assert_eq!(out.rosters_verified, out.links * 2);
    }
    for (i, check) in out.checks.iter().enumerate() {
        eprintln!(
            "node {} present={:?} absent={:?} corr={:.3}",
            labels[i], check.present_rel, check.absent_rel, check.correlation
        );
        assert_eq!(check.present_rel.len(), n - 1);
        for p in &check.present_rel {
            assert!(*p >= MixCheck::PRESENT_MIN_REL);
        }
        assert!(check.correlation >= MixCheck::CORR_MIN);
        let c = &out.counters[i];
        assert_eq!(c.tamper_or_auth_fail, 0);
        assert_eq!(c.replay, 0);
        assert_eq!(c.wrong_media_mode, 0);
        assert!(c.sent >= (MESH_DEFAULT_FRAMES * (n - 1)) as u64);
    }
}

#[tokio::test]
async fn three_peer_encrypted_mesh() {
    let labels = [LABEL_BOB, LABEL_ALICE, LABEL_LAPTOP];
    let out = host_mesh_roundtrip(&labels, true).await.expect("mesh3");
    assert_outcome(&labels, true, &out);
}

#[tokio::test]
async fn four_peer_encrypted_mesh() {
    let labels = [LABEL_BOB, LABEL_ALICE, LABEL_LAPTOP, LABEL_CHARLENE];
    let out = host_mesh_roundtrip(&labels, true).await.expect("mesh4");
    assert_eq!(out.links, 6);
    assert_outcome(&labels, true, &out);
}

#[tokio::test]
async fn three_peer_unencrypted_mesh_after_explicit_confirm() {
    let labels = [LABEL_BOB, LABEL_ALICE, LABEL_LAPTOP];
    let out = host_mesh_roundtrip(&labels, false).await.expect("plain3");
    assert_outcome(&labels, false, &out);
}

#[tokio::test]
async fn leave_continues_for_remaining_peers() {
    let labels = [LABEL_BOB, LABEL_ALICE, LABEL_LAPTOP, LABEL_CHARLENE];
    let mut mesh = HostMesh::establish(&labels, true).await.expect("mesh");

    // Phase 1: everyone present.
    let mixed = mesh.run(40).await.expect("phase1");
    for i in 0..labels.len() {
        mesh.check_node_mix(i, &mixed[i], PRIME)
            .verify()
            .unwrap_or_else(|e| panic!("phase1 {}: {e:#}", labels[i]));
    }

    // Peer 1 (the "starter" role is irrelevant) leaves; the call continues.
    mesh.leave(1).expect("leave");
    assert_eq!(mesh.live_indices(), vec![0, 2, 3]);
    for i in [0usize, 2, 3] {
        assert_eq!(mesh.link_count(i), 2);
    }
    assert!(mesh.leave(1).is_err(), "double leave must fail");

    // Phase 2: remaining three keep exchanging; departed tone is gone.
    let mixed = mesh.run(40).await.expect("phase2");
    assert!(mixed[1].is_empty());
    for i in [0usize, 2, 3] {
        mesh.check_node_mix(i, &mixed[i], 5)
            .verify()
            .unwrap_or_else(|e| panic!("phase2 {}: {e:#}", labels[i]));
    }

    // A second leave still leaves a working pair.
    mesh.leave(3).expect("leave 2");
    let mixed = mesh.run(40).await.expect("phase3");
    for i in [0usize, 2] {
        mesh.check_node_mix(i, &mixed[i], 5)
            .verify()
            .unwrap_or_else(|e| panic!("phase3 {}: {e:#}", labels[i]));
    }
}

#[tokio::test]
async fn fifth_participant_refused() {
    let labels = [
        LABEL_BOB,
        LABEL_ALICE,
        LABEL_LAPTOP,
        LABEL_CHARLENE,
        "15550100099",
    ];
    let err = match host_mesh_roundtrip(&labels, true).await {
        Ok(_) => panic!("five participants must be refused"),
        Err(e) => e,
    };
    let full = err
        .downcast_ref::<MeshFullError>()
        .expect("typed MeshFullError");
    assert_eq!(full.max, 4);
    assert_eq!(full.requested, 5);
    assert!(err.to_string().contains("limited to 4 participants"));
}
