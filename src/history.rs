use directories::ProjectDirs;
use rusqlite::{params, Connection};
use std::{error::Error, fs};

const SCHEMA: &str = "
    create table if not exists runs (
        id integer primary key,
        created_at text not null default current_timestamp,
        host text not null,
        database text not null,
        query_name text not null,
        sql text not null
    );
    create table if not exists timings (
        run_id integer not null references runs(id) on delete cascade,
        n integer not null,
        ms real not null
    );
    pragma user_version = 1;
";

fn open() -> Result<Connection, Box<dyn Error>> {
    let dirs = ProjectDirs::from("", "", "pglap").ok_or("no home directory")?;
    fs::create_dir_all(dirs.data_dir())?;
    let connection = Connection::open(dirs.data_dir().join("history.db"))?;
    connection.execute_batch(SCHEMA)?;
    Ok(connection)
}

pub fn save(host: &str, database: &str, query_name: &str, sql: &str, times: &[f64]) -> Result<(), Box<dyn Error>> {
    let mut connection = open()?;
    let transaction = connection.transaction()?;
    transaction.execute(
        "insert into runs (host, database, query_name, sql) values (?1, ?2, ?3, ?4)",
        params![host, database, query_name, sql],
    )?;
    let run_id = transaction.last_insert_rowid();
    for (n, ms) in times.iter().enumerate() {
        transaction.execute("insert into timings values (?1, ?2, ?3)", params![run_id, n as i64 + 1, ms])?;
    }
    transaction.commit()?;
    Ok(())
}

pub struct Run {
    pub created_at: String,
    pub database: String,
    pub times: Vec<f64>,
}

pub fn list(query_name: &str) -> Result<Vec<Run>, Box<dyn Error>> {
    let connection = open()?;
    let mut timings = connection.prepare("select ms from timings where run_id = ?1 order by n")?;
    let mut runs =
        connection.prepare("select id, created_at, database from runs where query_name = ?1 order by id desc")?;
    let rows = runs.query_map([query_name], |row| {
        let times = timings.query_map([row.get::<_, i64>(0)?], |row| row.get(0))?.collect::<Result<_, _>>()?;
        Ok(Run { created_at: row.get(1)?, database: row.get(2)?, times })
    })?;
    Ok(rows.collect::<Result<_, _>>()?)
}
