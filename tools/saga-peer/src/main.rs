//! saga-peer — Linux CLI reference peer for Saga voice (dev/test).

use std::path::PathBuf;

use anyhow::Result;
use clap::{Parser, Subcommand};
use saga_peer::call::{run_call, run_listen, CallOpts};
use saga_peer::identity::LABEL_LAPTOP;

#[derive(Parser, Debug)]
#[command(name = "saga-peer", about = "Saga Linux reference peer (dev/test)")]
struct Cli {
    #[command(subcommand)]
    cmd: Command,
}

#[derive(Subcommand, Debug)]
enum Command {
    /// Bind, register with relay, wait for an incoming Saga call.
    Listen {
        #[command(flatten)]
        common: CommonOpts,
        /// Prompt before answering (default: auto-answer).
        #[arg(long)]
        ask: bool,
    },
    /// Dial a peer by phone-digit label or Iroh EndpointId.
    Call {
        #[command(flatten)]
        common: CommonOpts,
        #[arg(long)]
        peer: String,
    },
    /// List cpal input/output devices (requires --features live-audio).
    Devices,
}

#[derive(clap::Args, Debug)]
struct CommonOpts {
    #[arg(long, default_value = LABEL_LAPTOP)]
    identity: String,
    /// Local relay URL (default http://127.0.0.1:3340). Ignored with --public-relay.
    #[arg(long)]
    relay: Option<String>,
    /// Use public N0 relays instead of the local dev relay.
    #[arg(long)]
    public_relay: bool,
    #[arg(long)]
    wav_in: Option<PathBuf>,
    #[arg(long)]
    wav_out: Option<PathBuf>,
    /// Loop --wav-in until hang-up.
    #[arg(long)]
    loop_wav: bool,
    /// Stop after N seconds (useful for scripted host tests).
    #[arg(long)]
    max_secs: Option<u64>,
    #[arg(long)]
    input_device: Option<String>,
    #[arg(long)]
    output_device: Option<String>,
}

fn main() -> Result<()> {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();
    let cli = Cli::parse();
    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?;

    match cli.cmd {
        Command::Devices => {
            #[cfg(feature = "live-audio")]
            {
                saga_peer::audio::list_devices()
            }
            #[cfg(not(feature = "live-audio"))]
            {
                anyhow::bail!("rebuild with --features live-audio to list cpal devices")
            }
        }
        Command::Listen { common, ask } => {
            let opts = to_opts(common, None, ask);
            let out = rt.block_on(run_listen(opts))?;
            eprintln!("done security={}", out.security);
            Ok(())
        }
        Command::Call { common, peer } => {
            let opts = to_opts(common, Some(peer), false);
            let out = rt.block_on(run_call(opts))?;
            eprintln!("done security={}", out.security);
            Ok(())
        }
    }
}

fn to_opts(common: CommonOpts, peer: Option<String>, ask: bool) -> CallOpts {
    CallOpts {
        identity: common.identity,
        peer,
        relay: common.relay,
        public_relay: common.public_relay,
        ask,
        wav_in: common.wav_in,
        wav_out: common.wav_out,
        loop_wav: common.loop_wav,
        force_plain: false,
        max_secs: common.max_secs,
        input_device: common.input_device,
        output_device: common.output_device,
    }
}
