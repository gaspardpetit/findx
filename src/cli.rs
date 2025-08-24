use camino::Utf8PathBuf;
use clap::{Args, Parser, Subcommand, ValueEnum};

#[derive(Parser, Debug)]
#[command(
    name = "findx",
    version = env!("FINDX_VERSION"),
    about = "Local document indexer",
    after_help = "Examples:\n  findx index\n  findx watch\n  findx query rust cli"
)]
pub struct Cli {
    #[arg(long, global = true, value_name = "FILE", default_value = "findx.toml")]
    pub config: Utf8PathBuf,

    #[arg(long, global = true, value_enum, default_value = "text")]
    pub log_format: LogFormat,

    #[command(subcommand)]
    pub command: Command,
}

#[derive(Copy, Clone, Debug, ValueEnum)]
pub enum LogFormat {
    Text,
    Json,
}

#[derive(Subcommand, Debug)]
pub enum Command {
    #[command(
        about = "Index files",
        long_about = "Index files. Defaults to the current directory and stores index data under .findx/.\n\nExamples:\n  findx index\n  findx index --roots src,docs"
    )]
    Index(IndexArgs),
    #[command(
        about = "Watch for changes",
        long_about = "Watch the filesystem for changes and keep the index updated. Uses the same defaults as the index command.\n\nExample:\n  findx watch"
    )]
    Watch(WatchArgs),
    #[command(
        about = "Query the index",
        long_about = "Search indexed documents. If no index is found, one is created automatically.\n\nExample:\n  findx query rust cli"
    )]
    Query(QueryArgs),
    Oneshot(OneshotArgs),
    #[command(about = "Serve HTTP API (not yet implemented)")]
    Serve(ServeArgs),
    #[command(about = "Apply database migrations (not yet implemented)")]
    Migrate(MigrateArgs),
    #[command(about = "Show indexing status (not yet implemented)")]
    Status,
}

#[derive(Args, Debug, Default)]
pub struct IndexArgs {
    #[arg(long, value_delimiter = ',', value_name = "PATHS")]
    pub roots: Vec<Utf8PathBuf>,

    #[arg(long, value_name = "FILE")]
    pub db: Option<Utf8PathBuf>,

    #[arg(long, value_name = "DIR", name = "tantivy-index")]
    pub tantivy_index: Option<Utf8PathBuf>,

    #[arg(long, value_name = "CMD")]
    pub extractor_cmd: Option<String>,
}

#[derive(Args, Debug, Default)]
pub struct WatchArgs {
    #[command(flatten)]
    pub index: IndexArgs,

    #[arg(long, value_name = "N", default_value_t = num_cpus::get())]
    pub threads: usize,
}

#[derive(Args, Debug, Default)]
pub struct QueryArgs {
    #[arg(long, value_name = "FILE")]
    pub db: Option<Utf8PathBuf>,

    #[arg(long, value_name = "DIR", name = "tantivy-index")]
    pub tantivy_index: Option<Utf8PathBuf>,

    #[arg(long, value_enum, default_value = "hybrid")]
    pub mode: QueryMode,

    #[arg(long, default_value_t = 20)]
    pub top_k: usize,

    #[arg(value_name = "QUERY")]
    pub query: String,

    #[arg(long, default_value_t = false)]
    pub chunks: bool,
}

#[derive(Copy, Clone, Debug, ValueEnum)]
pub enum QueryMode {
    Keyword,
    Semantic,
    Hybrid,
}

impl Default for QueryMode {
    fn default() -> Self {
        QueryMode::Hybrid
    }
}

#[derive(Args, Debug, Default)]
pub struct OneshotArgs {
    #[command(flatten)]
    pub index: IndexArgs,

    #[command(flatten)]
    pub query: QueryArgs,
}

#[derive(Args, Debug, Default)]
pub struct ServeArgs {
    #[arg(long, default_value = "127.0.0.1:8080")]
    pub bind: String,
}

#[derive(Args, Debug, Default)]
pub struct MigrateArgs {
    #[arg(long)]
    pub check: bool,

    #[arg(long)]
    pub apply: bool,
}
