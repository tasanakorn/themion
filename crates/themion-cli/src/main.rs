mod config;
use config::Config;
use themion_core::agent::{Agent, TurnStats};
use themion_core::client::OpenRouterClient;
use std::io::{self, BufRead, Write};

fn format_duration(ms: u128) -> String {
    if ms < 1000 {
        format!("{}ms", ms)
    } else if ms < 60_000 {
        format!("{:.1}s", ms as f64 / 1000.0)
    } else {
        format!("{}m{}s", ms / 60_000, (ms % 60_000) / 1000)
    }
}

fn format_stats(s: &TurnStats) -> String {
    format!("[stats: rounds={} tools={} in={} out={} cached={} time={}]",
        s.llm_rounds, s.tool_calls, s.tokens_in, s.tokens_out, s.tokens_cached, format_duration(s.elapsed_ms))
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let cfg = Config::load()?;

    let args: Vec<String> = std::env::args().skip(1).collect();

    if !args.is_empty() {
        // Print mode: send args as prompt, run agent loop, print result, exit
        let prompt = args.join(" ");
        let client = OpenRouterClient::new(cfg.api_key);
        let mut agent = Agent::new(client, cfg.model, cfg.system_prompt);
        let (result, stats) = agent.run_loop(&prompt).await?;
        println!("{result}");
        eprintln!("{}", format_stats(&stats));
    } else {
        // REPL mode
        let stdin = io::stdin();
        let client = OpenRouterClient::new(cfg.api_key);
        let mut agent = Agent::new_verbose(client, cfg.model.clone(), cfg.system_prompt);

        println!(
            "themion v{} | OpenRouter | {}",
            env!("CARGO_PKG_VERSION"),
            cfg.model
        );
        println!("Type '/exit' or '/quit' to quit.\n");

        loop {
            print!("> ");
            io::stdout().flush()?;

            let mut line = String::new();
            match stdin.lock().read_line(&mut line) {
                Ok(0) => break, // EOF
                Ok(_) => {}
                Err(e) => {
                    eprintln!("Error reading input: {e}");
                    break;
                }
            }

            let input = line.trim();
            if input.is_empty() {
                continue;
            }
            if input == "/exit" || input == "/quit" {
                break;
            }

            match agent.run_loop(input).await {
                Ok((response, stats)) => {
                    println!("{response}");
                    println!("{}", format_stats(&stats));
                }
                Err(e) => eprintln!("Error: {e}"),
            }
        }
    }

    Ok(())
}
