//! `cos questions`, `cos answer`, `cos say` and `cos report`: talking with a
//! run's chief.

use std::process::ExitCode;

use anyhow::bail;

use crate::client::Client;
use crate::home::CosHome;
use crate::runs_cmd::{Which, current_run};

#[derive(clap::Args)]
pub(crate) struct QuestionsArgs {
    /// Also list answered questions.
    #[arg(long)]
    all: bool,
}

#[derive(clap::Args)]
pub(crate) struct AnswerArgs {
    /// The question's id, from `cos questions`.
    id: i64,
    /// Your answer.
    answer: String,
}

#[derive(clap::Args)]
pub(crate) struct SayArgs {
    /// `[run] "text"`: the run defaults to the current one.
    #[arg(num_args = 1..=2, required = true, value_name = "RUN] [TEXT")]
    args: Vec<String>,
}

#[derive(clap::Args)]
pub(crate) struct ReportArgs {
    /// Run id. Defaults to the current run.
    run: Option<i64>,
}

pub(crate) async fn questions(home: &CosHome, args: QuestionsArgs) -> anyhow::Result<ExitCode> {
    let client = Client::new(home.socket());
    let questions = client
        .questions(if args.all { None } else { Some("open") })
        .await?;
    if questions.is_empty() {
        println!("No questions waiting for you.");
        return Ok(ExitCode::SUCCESS);
    }
    for question in &questions {
        println!("#{} (run {}) {}", question.id, question.run_id, question.question);
        if !question.choices.is_empty() {
            println!("    choices: {}", question.choices.join(" | "));
        }
        if let Some(answer) = &question.answer {
            println!("    answered: {answer}");
        }
    }
    if !args.all {
        println!("Answer with `cos answer <id> \"…\"`.");
    }
    Ok(ExitCode::SUCCESS)
}

pub(crate) async fn answer(home: &CosHome, args: AnswerArgs) -> anyhow::Result<ExitCode> {
    let client = Client::new(home.socket());
    let question = client.answer_question(args.id, args.answer).await?;
    println!(
        "Answered question {}; the chief of run {} gets it at its next wakeup.",
        question.id, question.run_id
    );
    Ok(ExitCode::SUCCESS)
}

pub(crate) async fn say(home: &CosHome, args: SayArgs) -> anyhow::Result<ExitCode> {
    let client = Client::new(home.socket());
    let (run, text) = match args.args.as_slice() {
        [text] => (current_run(&client, Which::Unfinished).await?, text.clone()),
        [run, text] => match run.parse::<i64>() {
            Ok(run) => (run, text.clone()),
            Err(_) => bail!("`{run}` isn't a run id; use `cos say [run] \"text\"`"),
        },
        _ => bail!("use `cos say [run] \"text\"`"),
    };
    client.message_chief(run, text).await?;
    println!("Sent to the chief of run {run}; it reads it at its next wakeup.");
    Ok(ExitCode::SUCCESS)
}

pub(crate) async fn report(home: &CosHome, args: ReportArgs) -> anyhow::Result<ExitCode> {
    let client = Client::new(home.socket());
    let run = match args.run {
        Some(run) => run,
        None => current_run(&client, Which::Any).await?,
    };
    let report = client.report(run).await?;
    print!("{}", report.markdown);
    if !report.markdown.ends_with('\n') {
        println!();
    }
    Ok(ExitCode::SUCCESS)
}
