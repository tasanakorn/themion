mod config;
use config::Config;
use themion_core::agent::Agent;
use themion_core::client::OpenRouterClient;
use std::io::{self, BufRead, Write};

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
        eprintln!("[rounds={} tools={} in={} out={} cached={}]",
                  stats.llm_rounds, stats.tool_calls, stats.tokens_in, stats.tokens_out, stats.tokens_cached);
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
                    println!("[rounds={} tools={} in={} out={} cached={}]",
                             stats.llm_rounds, stats.tool_calls, stats.tokens_in, stats.tokens_out, stats.tokens_cached);
                }
                Err(e) => eprintln!("Error: {e}"),
            }
        }
    }

    Ok(())
}
