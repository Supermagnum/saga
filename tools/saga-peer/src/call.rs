//! Voice ALPN handshake + RoQ media session for saga-peer.

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::{bail, Context, Result};
use iroh::endpoint::presets;
use iroh::tls::CaTlsConfig;
use iroh::{Endpoint, EndpointAddr, PublicKey, RelayMap, RelayMode, RelayUrl};
use iroh_roq::{Session as RoqSession, ALPN as ROQ_ALPN};
use log::{info, warn};
use mock_token::{
    decrypt_frame, encrypt_frame, run_initiator, run_responder, HandshakeOutcome, MockIdentity,
    SYNTHETIC_PLAINTEXT,
};
use saga_media::{
    CallRole, MediaCounters, MediaPipeline, SecurityResolution, FRAME_SAMPLES, SAMPLE_RATE,
};
use saga_session::HandshakeOutput;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::sync::Notify;

use crate::audio::{CaptureSource, PlayoutSink};
use crate::identity::{dev_secret_key, resolve_peer_label};

/// Same ALPN as Android `saga-iroh-core`.
pub const SAGA_VOICE_ALPN: &[u8] = b"saga/voice/1";

const DEFAULT_RELAY: &str = "http://127.0.0.1:3340";

#[derive(Clone, Debug)]
pub struct CallOpts {
    pub identity: String,
    pub peer: Option<String>,
    pub relay: Option<String>,
    pub public_relay: bool,
    pub ask: bool,
    pub wav_in: Option<PathBuf>,
    pub wav_out: Option<PathBuf>,
    pub loop_wav: bool,
    pub force_plain: bool,
    pub max_secs: Option<u64>,
    pub input_device: Option<String>,
    pub output_device: Option<String>,
}

pub struct PeerOutcome {
    pub security: &'static str,
    pub session_key_prefix: Option<(u8, u8)>,
    pub recorded: Vec<i16>,
    pub counters: Arc<MediaCounters>,
}

pub async fn run_listen(opts: CallOpts) -> Result<PeerOutcome> {
    let ep = bind_endpoint(&opts.identity, &opts).await?;
    info!(
        "listening as [{}] id=[{}] (waiting for Saga call)",
        opts.identity,
        ep.id()
    );
    ep.online().await;
    info!("relay online");

    loop {
        let incoming = ep.accept().await.context("accept")?;
        let conn = incoming.await.context("inbound handshake")?;
        if conn.alpn() != SAGA_VOICE_ALPN {
            // RoQ accept is handled inside media setup after voice.
            warn!("ignoring non-voice inbound ALPN {:?}", conn.alpn());
            continue;
        }
        if opts.ask {
            eprintln!(
                "Incoming call from {} — press Enter to answer",
                conn.remote_id()
            );
            let _ = tokio::task::spawn_blocking(|| {
                let mut line = String::new();
                let _ = std::io::stdin().read_line(&mut line);
            })
            .await;
        } else {
            info!("auto-answering from {}", conn.remote_id());
        }
        return drive_responder(ep, conn, opts).await;
    }
}

pub async fn run_call(opts: CallOpts) -> Result<PeerOutcome> {
    let peer = opts.peer.clone().context("--peer required for call")?;
    let ep = bind_endpoint(&opts.identity, &opts).await?;
    ep.online().await;
    let remote = resolve_endpoint_id(&peer)?;
    info!(
        "calling peer [{}] as [{}] → {}",
        peer, opts.identity, remote
    );
    let addr = EndpointAddr::from(remote);
    let conn = ep
        .connect(addr, SAGA_VOICE_ALPN)
        .await
        .context("voice connect")?;
    drive_initiator(ep, conn, peer, opts).await
}

async fn drive_initiator(
    ep: Endpoint,
    conn: iroh::endpoint::Connection,
    peer: String,
    opts: CallOpts,
) -> Result<PeerOutcome> {
    eprintln!("Securing…");
    let identity = MockIdentity::from_label(&opts.identity);
    let peer_label = resolve_peer_label(&peer);
    let local_eid = *ep.id().as_bytes();
    let remote_eid = *conn.remote_id().as_bytes();
    let (mut send, mut recv) = conn.open_bi().await.context("open_bi")?;
    let outcome = run_initiator(
        &identity,
        &peer_label,
        &local_eid,
        &remote_eid,
        false,
        &mut recv,
        &mut send,
    )
    .await;
    let (hs, security, prefix) = match outcome {
        HandshakeOutcome::Encrypted { output } => {
            let prefix = output.session_key_debug_prefix();
            #[cfg(debug_assertions)]
            eprintln!(
                "Encrypted (session_key_hash_prefix={:02x}{:02x})",
                prefix.0, prefix.1
            );
            #[cfg(not(debug_assertions))]
            eprintln!("Encrypted");
            let _ = send_media_probe(&mut send, &output).await;
            let _ = send.finish();
            (Some(output), "Encrypted", Some(prefix))
        }
        HandshakeOutcome::Failed { reason } => {
            warn!("handshake failed: {reason}");
            eprintln!("Not encrypted");
            let _ = send.finish();
            (None, "Not encrypted", None)
        }
    };

    if opts.force_plain && hs.is_some() {
        bail!("--force-plain cannot be used after a successful handshake");
    }

    let remote = conn.remote_id();
    // Keep voice connection alive while opening RoQ.
    let _voice = conn;
    let roq_conn = ep
        .connect(EndpointAddr::from(remote), ROQ_ALPN)
        .await
        .context("roq connect")?;
    run_media(
        ep,
        roq_conn,
        CallRole::Caller,
        hs,
        security,
        prefix,
        opts,
        true,
    )
    .await
}

async fn drive_responder(
    ep: Endpoint,
    conn: iroh::endpoint::Connection,
    opts: CallOpts,
) -> Result<PeerOutcome> {
    eprintln!("Securing…");
    let identity = MockIdentity::from_label(&opts.identity);
    let local_eid = *ep.id().as_bytes();
    let remote_eid = *conn.remote_id().as_bytes();
    let (mut send, mut recv) = conn.accept_bi().await.context("accept_bi")?;
    let outcome = run_responder(
        &identity,
        &local_eid,
        &remote_eid,
        false,
        &mut recv,
        &mut send,
    )
    .await;
    let (hs, security, prefix) = match outcome {
        HandshakeOutcome::Encrypted { output } => {
            let prefix = output.session_key_debug_prefix();
            #[cfg(debug_assertions)]
            eprintln!(
                "Encrypted (session_key_hash_prefix={:02x}{:02x})",
                prefix.0, prefix.1
            );
            #[cfg(not(debug_assertions))]
            eprintln!("Encrypted");
            let media_ok = read_media_probe(&mut recv, &output).await.unwrap_or(false);
            info!("media probe ok={media_ok}");
            (Some(output), "Encrypted", Some(prefix))
        }
        HandshakeOutcome::Failed { reason } => {
            warn!("handshake failed: {reason}");
            eprintln!("Not encrypted");
            (None, "Not encrypted", None)
        }
    };

    // Accept RoQ from initiator (same peer).
    let remote = conn.remote_id();
    let _voice = conn;
    let roq_conn = loop {
        let incoming = ep.accept().await.context("accept roq")?;
        let c = incoming.await.context("roq inbound")?;
        if c.alpn() == ROQ_ALPN && c.remote_id() == remote {
            break c;
        }
        warn!(
            "skipping inbound ALPN {:?} from {}",
            c.alpn(),
            c.remote_id()
        );
    };
    run_media(
        ep,
        roq_conn,
        CallRole::Callee,
        hs,
        security,
        prefix,
        opts,
        true,
    )
    .await
}

/// In-process host test helper: two endpoints, full handshake + RoQ media, wav A→B.
pub async fn host_pair_roundtrip(
    wav_pcm: &[i16],
    encrypted: bool,
) -> Result<(Vec<i16>, Arc<MediaCounters>, Arc<MediaCounters>)> {
    let label_a = "15550100011";
    let label_b = "15550100010";
    let ep_a = Endpoint::builder(presets::Minimal)
        .alpns(vec![SAGA_VOICE_ALPN.to_vec(), ROQ_ALPN.to_vec()])
        .secret_key(dev_secret_key(label_a))
        .relay_mode(RelayMode::Disabled)
        .bind()
        .await?;
    let ep_b = Endpoint::builder(presets::Minimal)
        .alpns(vec![SAGA_VOICE_ALPN.to_vec(), ROQ_ALPN.to_vec()])
        .secret_key(dev_secret_key(label_b))
        .relay_mode(RelayMode::Disabled)
        .bind()
        .await?;

    let addr_b = ep_b.addr();
    let accept = tokio::spawn({
        let ep_b = ep_b.clone();
        async move {
            // Voice
            let incoming = ep_b.accept().await.expect("voice accept").await.expect("v");
            assert_eq!(incoming.alpn(), SAGA_VOICE_ALPN);
            let identity = MockIdentity::from_label(label_b);
            let local_eid = *ep_b.id().as_bytes();
            let remote_eid = *incoming.remote_id().as_bytes();
            let (mut send, mut recv) = incoming.accept_bi().await.expect("bi");
            let outcome = run_responder(
                &identity,
                &local_eid,
                &remote_eid,
                !encrypted,
                &mut recv,
                &mut send,
            )
            .await;
            let hs = match outcome {
                HandshakeOutcome::Encrypted { output } => {
                    let _ = read_media_probe(&mut recv, &output).await;
                    Some(output)
                }
                HandshakeOutcome::Failed { .. } => None,
            };
            let remote = incoming.remote_id();
            let _voice = incoming;
            // RoQ
            let roq = loop {
                let i = ep_b.accept().await.expect("roq accept").await.expect("r");
                if i.alpn() == ROQ_ALPN && i.remote_id() == remote {
                    break i;
                }
            };
            (ep_b, roq, hs)
        }
    });

    let voice = ep_a
        .connect(addr_b.clone(), SAGA_VOICE_ALPN)
        .await
        .context("voice")?;
    let identity = MockIdentity::from_label(label_a);
    let local_eid = *ep_a.id().as_bytes();
    let remote_eid = *voice.remote_id().as_bytes();
    let (mut send, mut recv) = voice.open_bi().await?;
    let outcome = run_initiator(
        &identity,
        label_b,
        &local_eid,
        &remote_eid,
        !encrypted,
        &mut recv,
        &mut send,
    )
    .await;
    let hs_a = match outcome {
        HandshakeOutcome::Encrypted { output } => {
            let _ = send_media_probe(&mut send, &output).await;
            let _ = send.finish();
            Some(output)
        }
        HandshakeOutcome::Failed { .. } => {
            let _ = send.finish();
            None
        }
    };
    let remote = voice.remote_id();
    let _voice = voice;
    let roq_a = ep_a
        .connect(EndpointAddr::from(remote), ROQ_ALPN)
        .await
        .context("roq")?;
    let (_ep_b, roq_b, hs_b) = accept.await?;

    if encrypted {
        assert!(hs_a.is_some() && hs_b.is_some());
        assert_eq!(
            hs_a.as_ref().unwrap().saga_session_key.as_slice(),
            hs_b.as_ref().unwrap().saga_session_key.as_slice()
        );
    }

    let flow = MediaPipeline::flow_id_hint();
    let session_a = RoqSession::new(roq_a);
    let session_b = RoqSession::new(roq_b);
    let send_a = session_a.new_send_flow(flow).await?;
    let recv_a = session_a.new_receive_flow(flow).await?;
    let send_b = session_b.new_send_flow(flow).await?;
    let recv_b = session_b.new_receive_flow(flow).await?;
    let ca = Arc::new(MediaCounters::default());
    let cb = Arc::new(MediaCounters::default());

    let mut caller = if let Some(ref hs) = hs_a {
        MediaPipeline::start(
            SecurityResolution::Encrypted,
            Some(hs),
            CallRole::Caller,
            send_a,
            recv_a,
            ca.clone(),
        )?
    } else {
        MediaPipeline::start(
            SecurityResolution::NotEncrypted {
                prior_encrypted_history: false,
                user_continued_past_5a: false,
            },
            None,
            CallRole::Caller,
            send_a,
            recv_a,
            ca.clone(),
        )?
    };
    let mut callee = if let Some(ref hs) = hs_b {
        MediaPipeline::start(
            SecurityResolution::Encrypted,
            Some(hs),
            CallRole::Callee,
            send_b,
            recv_b,
            cb.clone(),
        )?
    } else {
        MediaPipeline::start(
            SecurityResolution::NotEncrypted {
                prior_encrypted_history: false,
                user_continued_past_5a: false,
            },
            None,
            CallRole::Callee,
            send_b,
            recv_b,
            cb.clone(),
        )?
    };

    let frames = wav_pcm.len() / FRAME_SAMPLES;
    let mut played = Vec::with_capacity(wav_pcm.len());
    for f in 0..frames {
        let mut frame = wav_pcm[f * FRAME_SAMPLES..(f + 1) * FRAME_SAMPLES].to_vec();
        caller.send_pcm_frame(&mut frame)?;
        tokio::time::timeout(Duration::from_secs(5), callee.recv_one_datagram())
            .await
            .context("recv timeout")??;
        let mut out = vec![0i16; FRAME_SAMPLES];
        let _ = callee.playout_frame(&mut out)?;
        played.extend_from_slice(&out);
    }
    for _ in 0..16 {
        let mut out = vec![0i16; FRAME_SAMPLES];
        let _ = callee.playout_frame(&mut out)?;
        played.extend_from_slice(&out);
    }

    drop(session_a);
    drop(session_b);
    drop(ep_a);
    Ok((played, ca, cb))
}

#[allow(clippy::too_many_arguments)]
async fn run_media(
    _ep: Endpoint,
    roq_conn: iroh::endpoint::Connection,
    role: CallRole,
    hs: Option<HandshakeOutput>,
    security: &'static str,
    prefix: Option<(u8, u8)>,
    opts: CallOpts,
    interactive: bool,
) -> Result<PeerOutcome> {
    let flow = MediaPipeline::flow_id_hint();
    let session = RoqSession::new(roq_conn);
    let send = session.new_send_flow(flow).await?;
    let recv = session.new_receive_flow(flow).await?;
    let counters = Arc::new(MediaCounters::default());

    let resolution = if hs.is_some() {
        SecurityResolution::Encrypted
    } else {
        SecurityResolution::NotEncrypted {
            prior_encrypted_history: false,
            user_continued_past_5a: true,
        }
    };
    let mut pipeline =
        MediaPipeline::start(resolution, hs.as_ref(), role, send, recv, counters.clone())?;

    let mut capture = if let Some(ref p) = opts.wav_in {
        CaptureSource::from_wav(p, opts.loop_wav)?
    } else {
        #[cfg(feature = "live-audio")]
        {
            CaptureSource::Live(crate::audio::LiveCapture::open(
                opts.input_device.as_deref(),
            )?)
        }
        #[cfg(not(feature = "live-audio"))]
        {
            let _ = opts.input_device;
            warn!("no --wav-in and live-audio feature disabled; sending silence");
            CaptureSource::Silence
        }
    };

    let mut sink = PlayoutSink::new(opts.wav_out.as_deref())?;
    #[cfg(feature = "live-audio")]
    {
        if opts.wav_in.is_none() || opts.output_device.is_some() {
            sink = sink.with_live(opts.output_device.as_deref())?;
        }
    }

    let stop = Arc::new(AtomicBool::new(false));
    let muted = Arc::new(AtomicBool::new(false));
    let notify = Arc::new(Notify::new());

    if interactive {
        let stop_c = stop.clone();
        let muted_c = muted.clone();
        let notify_c = notify.clone();
        tokio::spawn(async move {
            let mut stdin = tokio::io::BufReader::new(tokio::io::stdin());
            let mut buf = [0u8; 1];
            loop {
                match tokio::io::AsyncReadExt::read(&mut stdin, &mut buf).await {
                    Ok(0) => break,
                    Ok(_) => match buf[0] {
                        b'q' | b'Q' => {
                            stop_c.store(true, Ordering::SeqCst);
                            notify_c.notify_waiters();
                            break;
                        }
                        b'm' | b'M' => {
                            let now = !muted_c.load(Ordering::SeqCst);
                            muted_c.store(now, Ordering::SeqCst);
                            eprintln!("mute={}", now);
                        }
                        _ => {}
                    },
                    Err(_) => break,
                }
            }
        });
        let stop_c = stop.clone();
        let notify_c = notify.clone();
        tokio::spawn(async move {
            let _ = tokio::signal::ctrl_c().await;
            stop_c.store(true, Ordering::SeqCst);
            notify_c.notify_waiters();
        });
    }

    let deadline = opts
        .max_secs
        .map(|s| Instant::now() + Duration::from_secs(s));
    let frame_dur = Duration::from_millis(1000 * FRAME_SAMPLES as u64 / SAMPLE_RATE as u64);
    let mut last_stats = Instant::now();
    let mut tick = tokio::time::interval(frame_dur);

    loop {
        if stop.load(Ordering::SeqCst) {
            break;
        }
        if let Some(d) = deadline {
            if Instant::now() >= d {
                break;
            }
        }
        tick.tick().await;

        let mut frame = [0i16; FRAME_SAMPLES];
        let cont = capture.next_frame(&mut frame, muted.load(Ordering::Relaxed));
        if !cont && opts.wav_in.is_some() && !opts.loop_wav {
            // Drain a little more receive then stop.
            for _ in 0..25 {
                let _ =
                    tokio::time::timeout(Duration::from_millis(5), pipeline.recv_one_datagram())
                        .await;
                let mut out = [0i16; FRAME_SAMPLES];
                if pipeline.playout_frame(&mut out).is_ok() {
                    sink.push_frame(&out);
                }
            }
            break;
        }
        if let Err(e) = pipeline.send_pcm_frame(&mut frame) {
            warn!("send: {e}");
            break;
        }

        while tokio::time::timeout(Duration::from_millis(0), pipeline.recv_one_datagram())
            .await
            .is_ok()
        {}
        let mut out = [0i16; FRAME_SAMPLES];
        if pipeline.playout_frame(&mut out).is_ok() {
            sink.push_frame(&out);
        }

        if last_stats.elapsed() >= Duration::from_secs(1) {
            print_stats(&counters, &pipeline);
            last_stats = Instant::now();
        }
    }

    sink.finalize()?;
    drop(session);
    Ok(PeerOutcome {
        security,
        session_key_prefix: prefix,
        recorded: sink.recorded,
        counters,
    })
}

fn print_stats(counters: &MediaCounters, pipeline: &MediaPipeline) {
    let sent = counters.sent.load(Ordering::Relaxed);
    let played = counters.played.load(Ordering::Relaxed);
    let plc = counters.plc_frames.load(Ordering::Relaxed);
    let drop = counters.datagram_blocked_drops.load(Ordering::Relaxed);
    let tamper = counters.tamper_or_auth_fail.load(Ordering::Relaxed);
    let replay = counters.replay.load(Ordering::Relaxed);
    let wrong = counters.wrong_media_mode.load(Ordering::Relaxed);
    let loss = if sent + played == 0 {
        0.0
    } else {
        100.0 * plc as f64 / (played + plc).max(1) as f64
    };
    eprintln!(
        "stats sent={sent} played={played} loss≈{loss:.1}% jb={}/{} drops={drop} tamper={tamper} replay={replay} wrong_mode={wrong}",
        pipeline.jitter_len(),
        pipeline.jitter_cap()
    );
}

async fn bind_endpoint(identity: &str, opts: &CallOpts) -> Result<Endpoint> {
    let mut builder = Endpoint::builder(presets::N0)
        .alpns(vec![SAGA_VOICE_ALPN.to_vec(), ROQ_ALPN.to_vec()])
        .secret_key(dev_secret_key(identity));
    if opts.public_relay {
        // N0 defaults.
    } else {
        let url = opts
            .relay
            .clone()
            .unwrap_or_else(|| DEFAULT_RELAY.to_string());
        let relay_url: RelayUrl = url.parse().context("relay url")?;
        let map = RelayMap::from_iter([relay_url]);
        builder = builder
            .relay_mode(RelayMode::Custom(map))
            .ca_tls_config(CaTlsConfig::insecure_skip_verify());
    }
    Ok(builder.bind().await?)
}

fn resolve_endpoint_id(peer: &str) -> Result<PublicKey> {
    if let Ok(pk) = peer.parse::<PublicKey>() {
        return Ok(pk);
    }
    let label = resolve_peer_label(peer);
    Ok(dev_secret_key(&label).public())
}

async fn send_media_probe(
    writer: &mut (impl AsyncWriteExt + Unpin),
    output: &HandshakeOutput,
) -> Result<bool> {
    let key = *output.saga_session_key;
    let frame = encrypt_frame(&key, SYNTHETIC_PLAINTEXT);
    let len = (frame.len() as u32).to_be_bytes();
    writer.write_all(&len).await?;
    writer.write_all(&frame).await?;
    writer.flush().await?;
    Ok(true)
}

async fn read_media_probe(
    reader: &mut (impl AsyncReadExt + Unpin),
    output: &HandshakeOutput,
) -> Result<bool> {
    let key = *output.saga_session_key;
    let mut len_buf = [0u8; 4];
    reader.read_exact(&mut len_buf).await?;
    let len = u32::from_be_bytes(len_buf) as usize;
    let mut frame = vec![0u8; len];
    reader.read_exact(&mut frame).await?;
    let plain = decrypt_frame(&key, &frame).map_err(|e| anyhow::anyhow!("{e}"))?;
    Ok(plain == SYNTHETIC_PLAINTEXT)
}
