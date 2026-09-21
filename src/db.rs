use postgres::{CancelToken, Client, Config, Error, NoTls};
use std::sync::{mpsc::Sender, Mutex};
use std::{env, time::Instant};

pub fn host() -> String {
    env::var("PGHOST").unwrap_or("localhost".into())
}

fn connect(database: &str) -> Result<Client, Error> {
    let var = |name, default: &str| env::var(name).unwrap_or(default.into());
    Config::new()
        .host(&host())
        .port(var("PGPORT", "5432").parse().unwrap_or(5432))
        .user(&var("PGUSER", "postgres"))
        .password(var("PGPASSWORD", "postgres"))
        .dbname(database)
        .connect(NoTls)
}

pub fn list_databases() -> Result<Vec<String>, Error> {
    let sql = "select datname from pg_database where not datistemplate order by 1";
    let maintenance = env::var("PGDATABASE").unwrap_or("postgres".into());
    let rows = connect(&maintenance)?.query(sql, &[])?;
    Ok(rows.iter().map(|row| row.get(0)).collect())
}

pub fn benchmark(
    database: &str,
    sql: &str,
    runs: usize,
    progress: &Sender<String>,
    cancel: &Mutex<Option<CancelToken>>,
) -> Result<Vec<f64>, Error> {
    let mut client = connect(database)?;
    *cancel.lock().unwrap() = Some(client.cancel_token());
    let mut times = Vec::new();
    for run in 1..=runs {
        let start = Instant::now();
        client.simple_query(sql)?;
        let ms = start.elapsed().as_secs_f64() * 1000.0;
        times.push(ms);
        let _ = progress.send(format!("{run}/{runs}   last {ms:.2} ms"));
    }
    Ok(times)
}

pub fn message(error: Error) -> String {
    match error.as_db_error() {
        Some(db_error) => db_error.message().to_string(),
        None => error.to_string(),
    }
}

pub fn summary(mut times: Vec<f64>) -> String {
    times.sort_by(f64::total_cmp);
    let last = times.len() - 1;
    format!(
        "runs {}   min {:.2}   avg {:.2}   median {:.2}   p95 {:.2}   max {:.2}   (ms)",
        times.len(),
        times[0],
        times.iter().sum::<f64>() / times.len() as f64,
        times[last / 2],
        times[last * 95 / 100],
        times[last],
    )
}
