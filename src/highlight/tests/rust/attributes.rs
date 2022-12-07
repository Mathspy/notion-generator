#[derive(Parser, Serialize)]
struct Opts {
    #[clap(short, long, default_value = "partials/head.html")]
    head: PathBuf,
}
