use clap::Parser;
use dotenv::dotenv;

#[derive(Parser, Debug)]
#[command(author, version, about)]
pub struct Config {
    #[arg(short, long, env("HOOYAD_ENDPOINT"), default_value = "[::1]:8531")]
    pub endpoint: String,
}

impl Config {
    pub fn from_env_and_args() -> Self {
        dotenv().ok();
        Self::parse()
    }
}
