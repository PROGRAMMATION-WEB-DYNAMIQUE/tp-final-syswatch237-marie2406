use std::fmt;
use sysinfo::System;
use std::net::{TcpListener, TcpStream};
use std::io::{Read, Write};
use std::thread;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use std::fs::OpenOptions;
use chrono::Local;

use prettytable::{Table, Row, Cell};
use prettytable::row;

use std::io::{BufRead, BufReader}; // import regroupé proprement


// STRUCTURES DE DONNÉES


#[derive(Debug, Clone)]
struct CpuInfo {
    usage: f32,
}

#[derive(Debug, Clone)]
struct MemInfo {
    total: u64,
    used: u64,
}

#[derive(Debug, Clone)]
struct ProcessInfo {
    pid: i32,
    name: String,
    cpu: f32,
}

#[derive(Debug, Clone)]
struct SystemSnapshot {
    cpu: CpuInfo,
    mem: MemInfo,
    processes: Vec<ProcessInfo>,
}

// =========================
// DISPLAY (debug console)
// =========================

impl fmt::Display for CpuInfo {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "CPU Usage: {:.2}%", self.usage)
    }
}

impl fmt::Display for MemInfo {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "Memory: {}/{} MB", self.used, self.total)
    }
}

impl fmt::Display for ProcessInfo {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "{:<8} {:<20} {:>6.2}%", self.pid, self.name, self.cpu)
    }
}


// COLLECTE SYSTEME


fn collect_snapshot() -> SystemSnapshot {
    let mut sys = System::new_all();
    sys.refresh_all();

    // CPU global
    let cpu = CpuInfo {
        usage: sys.global_cpu_info().cpu_usage(),
    };

    // RAM (conversion KB → MB)
    let mem = MemInfo {
        total: sys.total_memory() / 1024,
        used: sys.used_memory() / 1024,
    };

    // Processus
    let mut processes: Vec<ProcessInfo> = sys.processes()
        .iter()
        .map(|(pid, p)| ProcessInfo {
            pid: pid.as_u32() as i32,
            name: p.name().to_string(),
            cpu: p.cpu_usage(),
        })
        .collect();

    // Tri CPU décroissant + limitation top 5
    processes.sort_by(|a, b| b.cpu.partial_cmp(&a.cpu).unwrap());
    processes.truncate(5);

    SystemSnapshot { cpu, mem, processes }
}


// TABLEAU PROPRE (CLI)


fn print_table(snapshot: &SystemSnapshot) {
    let mut table = Table::new();

    // Header
    table.add_row(row!["PID", "NAME", "CPU (%)"]);

    // Lignes processus
    for p in &snapshot.processes {
        table.add_row(Row::new(vec![
            Cell::new(&p.pid.to_string()),
            Cell::new(&p.name),
            Cell::new(&format!("{:.2}", p.cpu)),
        ]));
    }

    table.printstd();

    // Infos système
    println!("\nCPU : {:.2}%", snapshot.cpu.usage);
    println!("MEM : {}/{} MB", snapshot.mem.used, snapshot.mem.total);
}


// FORMAT REPONSE TCP


fn format_response(snapshot: &SystemSnapshot, command: &str) -> String {
    match command.trim() {
        "cpu" => format!("{}", snapshot.cpu),
        "mem" => format!("{}", snapshot.mem),

        // liste simple processus
        "ps" => snapshot.processes.iter()
            .map(|p| format!("{}", p))
            .collect::<Vec<_>>()
            .join("\n"),

        // tableau complet
        "all" => print_table_to_string(snapshot),

        "help" => String::from("Commands: cpu, mem, ps, all, help, quit"),

        "quit" => String::from("Bye!"),

        _ => String::from("Unknown command"),
    }
}


// TABLEAU POUR TCP 


fn print_table_to_string(snapshot: &SystemSnapshot) -> String {
    let mut table = Table::new();

    // HEADER OBLIGATOIRE
    table.add_row(row!["PID", "NAME", "CPU (%)"]);

    // DATA
    for p in &snapshot.processes {
        table.add_row(Row::new(vec![
            Cell::new(&p.pid.to_string()),
            Cell::new(&p.name),
            Cell::new(&format!("{:.2}", p.cpu)),
        ]));
    }

    // Capture propre en string
    let mut buffer = Vec::new();
    table.print(&mut buffer).unwrap();

    String::from_utf8_lossy(&buffer).to_string()
}


// GESTION CLIENT TCP


fn handle_client(stream: TcpStream, data: Arc<Mutex<SystemSnapshot>>) {
    let mut reader = BufReader::new(stream.try_clone().unwrap());
    let mut stream = stream;

    loop {
        let mut command = String::new();

        // lecture ligne client
        if reader.read_line(&mut command).is_err() {
            return;
        }

        let command = command.trim().to_string();

        if command.is_empty() {
            continue;
        }

        // snapshot partagé thread-safe
        let snapshot = {
            let data = data.lock().unwrap();
            data.clone()
        };

        // réponse
        let mut response = format_response(&snapshot, &command);

        // prompt interactif
        response.push_str("\n> ");

        // envoi réseau
        let _ = stream.write_all(response.as_bytes());
        let _ = stream.flush();

        // log fichier
        let mut file = OpenOptions::new()
            .append(true)
            .create(true)
            .open("syswatch.log")
            .unwrap();

        let log = format!(
            "[{}] Command: {}\n",
            Local::now(),
            command
        );

        let _ = file.write(log.as_bytes());

        // arrêt client
        if command == "quit" {
            let _ = stream.write_all(b"Bye!\n");
            let _ = stream.flush();
            break;
        }
    }
}


// MAIN SERVER


fn main() {
    let listener = TcpListener::bind("0.0.0.0:7878").unwrap();
    println!("Server running on port 7878");

    // état partagé système
    let data = Arc::new(Mutex::new(collect_snapshot()));

    // thread mise à jour automatique (CPU/RAM/process)
    thread::spawn({
        let data = Arc::clone(&data);
        move || loop {
            let mut snapshot = data.lock().unwrap();
            *snapshot = collect_snapshot();
            thread::sleep(Duration::from_secs(5));
        }
    });

    // serveur multi-clients
    for stream in listener.incoming() {
        match stream {
            Ok(stream) => {
                let data = Arc::clone(&data);
                thread::spawn(move || {
                    handle_client(stream, data);
                });
            }
            Err(e) => println!("Connection failed: {}", e),
        }
    }
}