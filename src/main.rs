use std::{io::Write, time::Duration};
use std::os::unix::fs::PermissionsExt;
use std::path::Path;

use actix_files::Files;
use actix_web::{
    App,
    Error,
    HttpResponse, HttpServer, middleware, web::{self, Json},
};
use actix_web::web::Bytes;
use lazy_static::lazy_static;
use serde::{Deserialize, Serialize};
use tokio::{task::JoinHandle, time::sleep};
use tokio_util::sync::CancellationToken;

lazy_static! {
    static ref API_URL: String = std::env::var("API_URL").unwrap_or("https://code.squid.pink/api/v1".to_string());

    static ref USER: String = std::env::var("VS_USER").unwrap();
    static ref SECRET_KEY: String = std::env::var("SECRET_KEY").unwrap();
    static ref API_KEY: String = format!("{}:{}", *USER, *SECRET_KEY);
}

fn init_tasks() -> (JoinHandle<()>, CancellationToken) {
    let cancel = CancellationToken::new();
    let c = cancel.clone();
    (
        tokio::spawn(async move {
            tokio::join!(
                poll_server(c.clone()),
                poll_heartbeat(c.clone())
            );
        }),
        cancel,
    )
}

// check the last modified date of the file /home/coder/.local/share/code-server/heartbeat
async fn poll_heartbeat(stop_signal: CancellationToken) {
    let client = reqwest::Client::new();
    let mut running = true;
    tokio::select! {
            _ = sleep(Duration::from_secs(60)) => {
                println!("Starting heartbeat check");
            }

            _ = stop_signal.cancelled() => {
                println!("gracefully shutting down cache purge job");
                return;
            }
        }

    loop {
        if let Ok(file) = std::fs::metadata("/home/coder/.local/share/code-server/heartbeat") {
            let last_modified = file.modified().unwrap();
            let now = std::time::SystemTime::now();
            let diff = now.duration_since(last_modified).unwrap();
            if diff.as_secs() > 60 && running {
                //  curl -d '{"action":"delete"}' -H 'Content-Type: application/json' -X POST https://code.squid.pink/api/v1/{USER}/
                let res = client
                    .post(&format!("{}/deployments/{}", *API_URL, *USER))
                    .json(&serde_json::json!({"action": "stop"}))
                    .header("Authorization", format!("{}", *API_KEY))
                    .send()
                    .await;

                match res {
                    Ok(res) => {
                        println!("[OK]\t[SHUTTING DOWN] {:?}", res);
                    }
                    Err(err) => {
                        eprintln!("[ERR]\t[SHUTTING DOWN] {:?}", err);
                    }
                }

                running = false;
            } else {
                println!("[OK]\t[VALID HEARTBEAT] heartbeat is good at {} seconds", diff.as_secs());
                running = true;
            }
        }
        tokio::select! {
            _ = sleep(Duration::from_secs(30)) => {
                continue;
            }

            _ = stop_signal.cancelled() => {
                println!("gracefully shutting down cache purge job");
                break;
            }
        }
    }
}

// Fetch file actions from the server
async fn poll_server(stop_signal: CancellationToken) {
    let client = reqwest::Client::new();
    loop {
        println!("fetching files from server with API KEY: {}", *API_KEY);
        let res = client
            .get(format!("{}/apps/file-sync/{}/", *API_URL, *USER).as_str())
            .header("Authorization", format!("{}", *API_KEY))
            .send()
            .await;

        match res {
            Ok(res) => {
                let body = res.text().await.unwrap();
                let files: Vec<File> =
                    match serde_json::from_str(&body) {
                        Ok(files) => files,
                        Err(err) => {
                            eprintln!("[ERR]\t[PARSING FILES] {:?}", err);
                            eprintln!("body: {}", body);
                            sleep(Duration::from_secs(10)).await;
                            continue;
                        }
                    };

                for file in files {
                    let full = format!("/home/coder/{}", file.path);
                    let path = Path::new(&full);
                    let prefix = path.parent().unwrap();
                    std::fs::create_dir_all(prefix).unwrap();

                    let response = reqwest::get(&file.url).await.unwrap();
                    let mut bytes = response.bytes().await.unwrap();
                    // If the file currently exists, check if it's the same as the one on the server
                    if path.exists() {
                        println!("[OK]\t[FILE EXISTS] {}", file.path);

                    } else {
                        let mut io_file = std::fs::File::create(path).unwrap();


                        // if file is json replace {VS_USER} with the actual user
                        if file.path.ends_with(".json") {
                            let mut content = String::from_utf8(bytes.to_vec()).unwrap();
                            content = content.replace("{VS_USER}", &*USER);
                            bytes = Bytes::from(content);
                        }
                        io_file.write_all(&mut bytes).unwrap();

                        let permissions = std::fs::metadata(path).unwrap().permissions();
                        // Allow everyone to read and write
                        if file.path.contains(".code") {
                            let perms = permissions.mode() | 0o744;
                            std::fs::set_permissions(path, std::fs::Permissions::from_mode(perms)).unwrap();
                        } else {
                            let perms = permissions.mode() | 0o666;
                            std::fs::set_permissions(path, std::fs::Permissions::from_mode(perms)).unwrap();
                        }
                    }
                //     sent a post request to /apps/file-sync/{User}/ with the data {file_id: file_id}
                    match client.post(&format!(
                        "{}/apps/file-sync/{}/",
                        *API_URL,
                        *USER
                    ))
                        .json(&serde_json::json!({"file_id": file.id}))
                        .header("Authorization", format!("{}", *API_KEY))
                        .send()
                        .await
                        .unwrap()
                        .json::<serde_json::Value>()
                        .await {
                            Ok(res) => {
                                println!("[OK]\t[FILE SYNCED] {:?}", res);
                            }
                            Err(err) => {
                                eprintln!("[ERR]\t[FILE SYNCED] {:?}", err);
                            }
                    }


                }
            }
            Err(err) => {
                eprintln!("[ERR]\t[FETCHING FILES] {:?}", err);
            }
        }

        tokio::select! {
            _ = sleep(Duration::from_secs(30)) => {
                continue;
            }

            _ = stop_signal.cancelled() => {
                println!("gracefully shutting down cache purge job");
                break;
            }
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct File {
    id: i32,
    url: String,
    path: String,
}

async fn save(file: Json<File>) -> Result<HttpResponse, Error> {
    let full = format!("/home/coder/{}", file.path);
    let path = Path::new(&full);
    let prefix = path.parent().unwrap();
    std::fs::create_dir_all(prefix).unwrap();

    let mut io_file = std::fs::File::create(path).unwrap();

    let response = reqwest::get(&file.url).await.unwrap();
    let mut bytes = response.bytes().await.unwrap();
    // if file is json replace {VS_USER} with the actual user
    if file.path.ends_with(".json") {
        let mut content = String::from_utf8(bytes.to_vec()).unwrap();
        content = content.replace("{VS_USER}", &*USER);
        bytes = Bytes::from(content);
    }
    io_file.write_all(&mut bytes).unwrap();

    let permissions = std::fs::metadata(path).unwrap().permissions();
    // Allow everyone to read and write
    if file.path.contains(".code") {
        let perms = permissions.mode() | 0o744;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(perms)).unwrap();
    } else {
        let perms = permissions.mode() | 0o666;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(perms)).unwrap();
    }

    Ok(HttpResponse::Ok().body("SUCCESS".to_string()))
}

async fn it_works() -> HttpResponse {
    HttpResponse::Ok().body("It works!")
}

#[actix_web::main]
async fn main() -> std::io::Result<()> {
    std::env::set_var("RUST_LOG", "info");
    println!("Starting server...");
    let (handle, cancel) = init_tasks();

    HttpServer::new(|| {
        App::new()
            .wrap(middleware::Logger::default())
            .service(web::resource("/").route(web::get().to(it_works)))
            .service(web::resource("/save").route(web::post().to(save)))
            .service(Files::new("/serve", "/home/coder").show_files_listing())
    })
        .bind(("0.0.0.0", 3000))?
        .run()
        .await?;

    cancel.cancel();

    handle.await.unwrap();

    Ok(())
}
