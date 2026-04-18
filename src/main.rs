mod agent;
mod client;
mod tools;

use agent::Agent;
use client::OpenRouterClient;
use std::io::{self, BufRead, Write};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let api_key = std::env::var("OPENROUTER_API_KEY")
        .expect("OPENROUTER_API_KEY env var is required");
    let model = std::env::var("OPENROUTER_MODEL")
        .unwrap_or_else(|_| "minimax/minimax-m2.7".to_string());
    let system_prompt = std::env::var("SYSTEM_PROMPT")
        .unwrap_or_else(|_| "You are a helpful AI assistant with access to tools.".to_string());

    let args: Vec<String> = std::env::args().skip(1).collect();

    if !args.is_empty() {
        // Print mode: send args as prompt, run agent loop, print result, exit
        let prompt = args.join(" ");
        let client = OpenRouterClient::new(api_key);
        let mut agent = Agent::new(client, model, system_prompt);
        let result = agent.run_loop(&prompt).await?;
        println!("{result}");
    } else {
        // REPL mode
        let stdin = io::stdin();
        let client = OpenRouterClient::new(api_key);
        let mut agent = Agent::new(client, model, system_prompt);

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
            if input == "exit" || input == "quit" {
                break;
            }

            match agent.run_loop(input).await {
                Ok(response) => println!("{response}"),
                Err(e) => eprintln!("Error: {e}"),
            }
        }
    }

    Ok(())
}
