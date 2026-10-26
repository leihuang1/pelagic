#![forbid(unsafe_code)]

use std::error::Error;
use std::io;
use std::net::SocketAddr;
use std::num::NonZeroUsize;
use std::time::Duration;

use bellhop::solver::SimulationLimits;
use clap::Parser;
use server::{ServerConfig, app};
use tracing_subscriber::EnvFilter;

#[derive(Debug, Parser)]
#[command(
    name = "pelagic-server",
    version,
    about = "Pelagic synchronous JSON/HDF5 HTTP service for BELLHOP"
)]
struct Args {
    /// Address on which to accept HTTP connections.
    #[arg(long, env = "BELLHOP_LISTEN", default_value = "0.0.0.0:8080")]
    listen: SocketAddr,

    /// Maximum number of requests admitted to blocking simulation work.
    #[arg(long, env = "BELLHOP_WORKERS", default_value_t = default_workers())]
    workers: usize,

    /// Threads for each admitted simulation. Results are identical for any count.
    #[arg(long, env = "BELLHOP_SOLVER_THREADS", default_value_t = NonZeroUsize::MIN)]
    solver_threads: NonZeroUsize,

    /// End-to-end request timeout, including time spent waiting for a worker.
    #[arg(long, env = "BELLHOP_REQUEST_TIMEOUT_SECONDS", default_value_t = 120)]
    request_timeout_seconds: u64,

    /// Maximum JSON request-body size.
    #[arg(long, env = "BELLHOP_MAX_BODY_BYTES", default_value_t = 16 * 1024 * 1024)]
    max_body_bytes: usize,

    /// Maximum serialized size of a JSON simulation response.
    #[arg(
        long,
        env = "BELLHOP_MAX_JSON_RESPONSE_BYTES",
        default_value_t = 64 * 1024 * 1024
    )]
    max_json_response_bytes: usize,

    /// Optional static bearer token. TLS must be terminated externally.
    #[arg(long, env = "BELLHOP_AUTH_TOKEN", hide_env_values = true)]
    auth_token: Option<String>,

    /// Maximum launch rays across all source depths.
    #[arg(long, env = "BELLHOP_MAX_RAYS", default_value_t = 1_000_000)]
    max_rays: usize,

    /// Maximum integration steps for any one ray.
    #[arg(long, env = "BELLHOP_MAX_STEPS_PER_RAY", default_value_t = 100_000)]
    max_steps_per_ray: usize,

    /// Maximum stored ray or eigenray trajectory points per result class.
    #[arg(long, env = "BELLHOP_MAX_TOTAL_POINTS", default_value_t = 20_000_000)]
    max_total_points: usize,

    /// Maximum arrivals retained for any one receiver.
    #[arg(
        long,
        env = "BELLHOP_MAX_ARRIVALS_PER_RECEIVER",
        default_value_t = 20_000_000
    )]
    max_arrivals_per_receiver: usize,

    /// Maximum arrivals retained across one complete simulation.
    #[arg(long, env = "BELLHOP_MAX_TOTAL_ARRIVALS", default_value_t = 20_000_000)]
    max_total_arrivals: usize,

    /// Maximum pressure-field receiver cells.
    #[arg(long, env = "BELLHOP_MAX_FIELD_CELLS", default_value_t = 20_000_000)]
    max_field_cells: usize,
}

fn default_workers() -> usize {
    std::thread::available_parallelism().map_or(1, std::num::NonZero::get)
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()))
        .init();
    let args = Args::parse();
    validate_args(&args)?;

    let limits = SimulationLimits {
        max_rays: args.max_rays,
        max_steps_per_ray: args.max_steps_per_ray,
        max_total_ray_points: args.max_total_points,
        max_arrivals_per_receiver: args.max_arrivals_per_receiver,
        max_total_arrivals: args.max_total_arrivals,
        max_total_eigenray_points: args.max_total_points,
        max_field_cells: args.max_field_cells,
        ..SimulationLimits::default()
    };
    let config = ServerConfig {
        workers: args.workers,
        request_timeout: Duration::from_secs(args.request_timeout_seconds),
        max_body_bytes: args.max_body_bytes,
        max_json_response_bytes: args.max_json_response_bytes,
        simulation_limits: limits,
        solver_threads: args.solver_threads,
        auth_token: args.auth_token,
    };

    let listener = tokio::net::TcpListener::bind(args.listen).await?;
    tracing::info!(
        address = %args.listen,
        workers = args.workers,
        solver_threads = args.solver_threads,
        "pelagic server listening"
    );
    axum::serve(listener, app(config))
        .with_graceful_shutdown(shutdown_signal())
        .await?;
    Ok(())
}

fn validate_args(args: &Args) -> Result<(), io::Error> {
    if args.request_timeout_seconds == 0 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "--request-timeout-seconds must be positive",
        ));
    }
    let values = [
        ("workers", args.workers),
        ("max-body-bytes", args.max_body_bytes),
        ("max-json-response-bytes", args.max_json_response_bytes),
        ("max-rays", args.max_rays),
        ("max-steps-per-ray", args.max_steps_per_ray),
        ("max-total-points", args.max_total_points),
        ("max-arrivals-per-receiver", args.max_arrivals_per_receiver),
        ("max-total-arrivals", args.max_total_arrivals),
        ("max-field-cells", args.max_field_cells),
    ];
    if let Some((name, _)) = values.into_iter().find(|(_, value)| *value == 0) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("--{name} must be positive"),
        ));
    }
    Ok(())
}

async fn shutdown_signal() {
    if let Err(error) = tokio::signal::ctrl_c().await {
        tracing::error!(%error, "unable to install shutdown signal handler");
    }
}
