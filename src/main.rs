use std::time::{Duration, Instant};
use std::{
    env, fs,
    io::{self, Write},
    path::{Path, PathBuf},
};

use anyhow::{anyhow, Context};
use chrono::{Local, NaiveDate, Weekday};
use clap::{Args, Parser, Subcommand};
use indoc::indoc;
use mac_notification_sys::{
    error::NotificationResult, get_bundle_identifier_or_default, set_application,
};
use notify::{Config, RecommendedWatcher, RecursiveMode, Watcher};
use notify_rust::Notification;
use serde::Deserialize;

use chrono::Datelike;
use harvest_openapi::apis::{
    configuration::{ApiKey, Configuration},
    default_api::{self as harvest, ListTimeEntriesParams},
};

// use notify::{Config, DebouncedEvent, RecommendedWatcher, RecursiveMode, Watcher};
// use std::sync::mpsc::{channel, Receiver};
// use std::time::Duration;

#[derive(Parser)]
#[command(author, version, about, long_about = None)]
struct Cli {
    #[command(subcommand)]
    command: Option<Commands>,
}

#[derive(Subcommand)]
enum Commands {
    /// prints timers for the most recent two weeks
    Report {},

    Watch(WatchArgs),

    TestNotification {},
}

#[derive(Args)]
pub struct WatchArgs {
    /// Project clockhand files to watch{n}
    /// example values:{n}
    /// - project_a/clockhand.json project_b/clockhand.json{n}
    /// - ~/code/*/clockhand.json (shell expansion)
    // #[arg(short, long)]
    // files: Vec<String>,
    #[clap(name = "project-config-paths")]
    project_config_paths: Vec<String>,

    /// interval in seconds that the notifications will occur at
    #[arg(short, long, default_value = "60")]
    interval: u64,
}

#[derive(Debug, Deserialize)]
struct AccessTokenConfig {
    token: String,
    account_id: i32,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();

    match &cli.command {
        Some(Commands::Watch(watch_args)) => {
            setup_notification_application()?;

            let token_data = access_token()?;

            let config = Configuration {
                bearer_access_token: Some(token_data.token),
                api_key: Some(ApiKey {
                    key: token_data.account_id.to_string(),
                    prefix: None,
                }),
                ..Default::default()
            };

            // Create a channel to receive file system events
            let (tx, rx) = std::sync::mpsc::channel();

            // Create a watcher object and register the directories to watch

            let mut watcher: RecommendedWatcher = Watcher::new(
                tx,
                Config::default().with_poll_interval(Duration::from_secs(2)),
            )
            .unwrap();

            let mut last_request_time = Instant::now()
                .checked_sub(Duration::from_secs(watch_args.interval))
                .unwrap();

            let mut projects: Vec<Project> = Vec::new();

            for clockhand_config_path in watch_args.project_config_paths.iter() {
                let clockhand_config_pathbuf = PathBuf::from(clockhand_config_path);
                let project = read_project_config(&clockhand_config_pathbuf)?;

                watcher
                    .watch(&project.root, RecursiveMode::Recursive)
                    .unwrap();

                println!("Watching {:?}", &project.root);
                projects.push(project);
            }

            // Start an event loop to process file system events
            loop {
                match rx.recv() {
                    Ok(e) => match e {
                        Ok(ee) => {
                            // println!("changed: {:?}", &ee);
                            // println!("info: {:?}", &ee.info());
                            // debouncer.put(String::from(ee.paths[0]))

                            let path = ee.paths.first();
                            println!(
                                "changed: {:?}, time since {:?}",
                                path,
                                last_request_time.elapsed()
                            );

                            if last_request_time.elapsed()
                                > Duration::from_secs(watch_args.interval)
                            {
                                println!("notifying!");
                                last_request_time = Instant::now();
                                notify_project_timer_status(path.unwrap(), &projects, &config)
                                    .await?;
                            } else {
                                println!("interval hasn't passed, not notifying");
                            }
                        }
                        Err(ee) => {
                            println!("watch error: {:?}", ee);
                        }
                    },
                    Err(e) => println!("watch error: {:?}", e),
                }
            }
        }
        Some(Commands::TestNotification {}) => {
            setup_notification_application()?;

            Notification::new()
                .summary(concat!("Test notification from ", env!("CARGO_PKG_NAME")))
                .body(&format!(
                    "This is a test notification at {}",
                    chrono::Local::now().to_rfc2822()
                ))
                .sound_name("Sosumi")
                .show()?;
        }

        Some(Commands::Report {}) => {
            let token_data = access_token()?;

            let config = Configuration {
                bearer_access_token: Some(token_data.token),
                api_key: Some(ApiKey {
                    key: token_data.account_id.to_string(),
                    prefix: None,
                }),
                ..Default::default()
            };

            let me = harvest::retrieve_the_currently_authenticated_user(&config)
                .await
                .unwrap();

            let now = Local::now();
            let today = now.date_naive();

            // iso week
            let iso_week = today.iso_week().week();

            // first monday of this week
            let start_of_week = NaiveDate::from_isoywd_opt(today.year(), iso_week, Weekday::Mon)
                .ok_or_else(|| anyhow!("could not determine first day of week"))?;

            let start_of_last_week = start_of_week - chrono::Duration::weeks(1);

            let timers = harvest::list_time_entries(
                &config,
                ListTimeEntriesParams {
                    user_id: me.id,
                    per_page: Some(200),
                    from: Some(start_of_last_week.to_string()),
                    ..Default::default()
                },
            )
            .await
            .unwrap();

            // loops through all the timers and prints them using the tabwriter crate
            let mut tw = tabwriter::TabWriter::new(io::stdout());

            timers.time_entries.into_iter().for_each(|timer| {
                let proj = timer.project.unwrap();
                writeln!(
                    &mut tw,
                    "{}\t{}\t{}\t{}\t{}",
                    timer.spent_date.unwrap(),
                    &proj.id.unwrap(),
                    strip_newlines_and_tabs(&proj.name.unwrap()),
                    decimal_hours_to_string(timer.hours.unwrap()),
                    truncate_with_ellipsis(&timer.notes.unwrap_or("(none)".to_string()), 60)
                )
                .unwrap();
            });

            tw.flush()?;
        }
        None => {}
    }

    Ok(())
}

fn config_dir() -> anyhow::Result<PathBuf> {
    let home = env::var("HOME")?;
    let path = Path::new(&home)
        .join(".config")
        .join(env!("CARGO_PKG_NAME"));
    Ok(path)
}

fn access_token() -> anyhow::Result<AccessTokenConfig> {
    let path = Path::new(&config_dir()?).join("access-token.json");
    let path_string = path
        .as_os_str()
        .to_str()
        .ok_or_else(|| anyhow!("failed to generate a path to access-token"))?;

    let token_file_contents = fs::read_to_string(&path).context(format!(
        indoc! {r#"
            didn't find the credentials config file at {}

            1. visit https://id.getharvest.com/developers
            2. Create new personal access token
            3. _TODO_ implement command to write json like {{"token": "123...", account_id: 456}}
               into this file: {}
        "#},
        path_string, path_string
    ))?;

    let token_data = serde_json::from_str::<AccessTokenConfig>(&token_file_contents)
        .with_context(|| format!("bad format for {}", path_string))?;

    Ok(token_data)
}

fn setup_notification_application() -> NotificationResult<()> {
    let app_bundle_id = get_bundle_identifier_or_default("Terminal");
    set_application(&app_bundle_id)?;
    Ok(())
}

/// removes all newlines and tabs from a string
fn strip_newlines_and_tabs(s: &str) -> String {
    s.replace("\t", "").replace("\n", "")
}

/// truncates a string to a given length based on word boundaries, and appends an ellipsis
fn truncate_with_ellipsis(s: &str, len: usize) -> String {
    let mut words = s.split_whitespace();
    let mut truncated = String::new();

    while let Some(word) = words.next() {
        if truncated.len() + word.len() > len {
            truncated.push_str("…");
            break;
        }

        truncated.push_str(word);
        truncated.push(' ');
    }

    truncated
}

/// # Examples
fn decimal_hours_to_string(decimal_hours: f32) -> String {
    let hours = decimal_hours.floor() as i32;
    let minutes = ((decimal_hours - hours as f32) * 60.0).round() as i32;
    if hours == 0 {
        format!("    {:02}m", minutes)
    } else {
        format!("{:02}h {:02}m", hours, minutes)
    }
}

#[derive(Debug, Deserialize)]
pub struct ProjectConfig {
    pub harvest_project_id: i32,
    pub name: String,
}

struct Project {
    pub harvest_project_id: i32,
    pub root: PathBuf,
    pub name: String,
}

impl Project {
    fn contains_file(&self, path: PathBuf) -> bool {
        path.starts_with(&self.root)
    }
}

fn read_project_config(path: &PathBuf) -> anyhow::Result<Project> {
    let path_string = path
        .as_os_str()
        .to_str()
        .ok_or_else(|| anyhow!("failed to generate a path to project config"))?;

    let project_file_contents = fs::read_to_string(&path).context(format!(
        indoc! {r#"
            didn't find the project config file at {}

            1. Create a file at this path with the following contents:
               {{"project_id": 12345}}
        "#},
        path_string
    ))?;

    let project_data = serde_json::from_str::<ProjectConfig>(&project_file_contents)
        .with_context(|| format!("bad format for {}", path_string))?;

    // parent directory of path
    let config_dir = path
        .parent()
        .ok_or_else(|| anyhow!("failed to get parent directory of path"))?
        .to_path_buf();

    // if the config_dir is a directory named ".config" then config_dir's parent, otherwise config_dig
    let project_dir = if config_dir
        .file_name()
        .ok_or_else(|| anyhow!("failed to get file name of config_dir"))?
        .to_str()
        .ok_or_else(|| anyhow!("failed to convert file name to string"))?
        == ".config"
    {
        config_dir
            .parent()
            .ok_or_else(|| anyhow!("failed to get parent directory of config_dir"))?
            .to_path_buf()
    } else {
        config_dir
    };

    Ok(Project {
        harvest_project_id: project_data.harvest_project_id,
        root: project_dir,
        name: project_data.name,
    })
}

// an enum with three possible states: true, timer not running, timer for different project
enum TimerStatus {
    TimerRunning,
    TimerNotRunning,
    TimerForDifferentProject,
}

async fn is_active_timer_for_project(
    config: &Configuration,
    project: &Project,
) -> anyhow::Result<TimerStatus> {
    // let active_timer = harvest_client.get_active_timer().await?;
    let me = harvest::retrieve_the_currently_authenticated_user(config)
        .await
        .unwrap();

    // let active_timers = harvest::list_time_entries(config).

    let running_timers = harvest::list_time_entries(
        &config,
        ListTimeEntriesParams {
            user_id: me.id,
            per_page: Some(1),
            is_running: Some(true),
            ..Default::default()
        },
    )
    .await?;

    let running_timer = running_timers.time_entries.first();

    match running_timer {
        Some(timer) => {
            let timer_project_id = timer.project.as_ref().unwrap().id.unwrap();
            if timer_project_id == project.harvest_project_id {
                return Ok(TimerStatus::TimerRunning);
            } else {
                return Ok(TimerStatus::TimerForDifferentProject);
            }
        }
        None => {
            return Ok(TimerStatus::TimerNotRunning);
        }
    }
}

async fn notify_project_timer_status(
    path: &PathBuf,
    projects: &Vec<Project>,
    config: &Configuration,
) -> anyhow::Result<()> {
    // what project was this file in?
    let project = projects
        .iter()
        .find(|p| p.contains_file(path.clone()))
        .ok_or_else(|| anyhow!("path isn't in any project"))
        .unwrap();

    // use the harvest api to determine if there's an active timer running
    let active_timer = is_active_timer_for_project(config, project).await?;

    match active_timer {
        TimerStatus::TimerRunning => {
            // noop
        }

        TimerStatus::TimerNotRunning => {
            Notification::new()
                .summary("Timer not running")
                .body(&format!("Start a timer for {}", project.name))
                .sound_name("Sosumi")
                .show()
                .unwrap();
        }
        TimerStatus::TimerForDifferentProject => {
            Notification::new()
                .summary("Timer running for other project")
                .body(&format!("Start a timer for {}", project.name))
                .sound_name("Sosumi")
                .show()
                .unwrap();
        }
    };

    Ok(())
}
