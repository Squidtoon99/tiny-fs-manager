use actix_files::Files;
use actix_web::{
    middleware,
    web::{self, Json},
    App, Error, HttpResponse, HttpServer,
};
use lazy_static::lazy_static;
use serde::{Deserialize, Serialize};
use std::{io::Write, time::Duration};
use tokio::{task::JoinHandle, time::sleep};
use tokio_util::sync::CancellationToken;

lazy_static! {
    static ref USER: String = std::env::var("USER").unwrap();
}

fn init_poll_heartbeat() -> (JoinHandle<()>, CancellationToken) {
    let cancel = CancellationToken::new();

    (
        tokio::spawn(poll_heartbeat(
            cancel.clone(),
        )),
        cancel,
    )
}
// check the last modified date of the file /home/coder/.local/share/code-server/heartbeat
async fn poll_heartbeat(stop_signal: CancellationToken) -> () {
    let client = reqwest::Client::new();
    loop {
        if let Ok(file) = std::fs::metadata("/home/coder/.local/share/code-server/heartbeat") {
            let last_modified = file.modified().unwrap();
            let now = std::time::SystemTime::now();
            let diff = now.duration_since(last_modified).unwrap();
            if diff.as_secs() > 60 {
                //  curl -d '{"action":"delete"}' -H 'Content-Type: application/json' -X POST https://code.squid.pink/api/v1/{USER}/
                let res = client
                    .post(&format!("https://code.squid.pink/api/v1/{}/", *USER))
                    .json(&serde_json::json!({"action": "delete"}))
                    .send()
                    .await;

                if let Ok(res) = res {
                    println!("res: {:?}", res);
                }
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
        };
    }
   () 
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct File {
    url: String,
    path: String,
}

async fn save(file: Json<File>) -> Result<HttpResponse, Error> {
    let folder = file.path.split("/").collect::<Vec<&str>>()[0];
    std::fs::create_dir_all(format!("/home/coder/{}", folder)).unwrap();
    let mut io_file = std::fs::File::create(format!("/home/coder/{}", file.path)).unwrap();

    let response = reqwest::get(&file.url).await.unwrap();
    let mut bytes = response.bytes().await.unwrap();
    io_file.write_all(&mut bytes).unwrap();

    Ok(HttpResponse::Ok().body(format!("{} {}", folder, file.url)))
}

async fn it_works() -> HttpResponse {
    HttpResponse::Ok().body("It works!")
}

#[actix_web::main]
async fn main() -> std::io::Result<()> {
    std::env::set_var("RUST_LOG", "info");

    let (handle, cancel) = init_poll_heartbeat();

    HttpServer::new(|| {
        App::new()
            .wrap(middleware::Logger::default())
            .service(web::resource("/").route(web::get().to(it_works)))
            .service(web::resource("/save").route(web::post().to(save)))
            .service(Files::new("/serve", "/home/coder").show_files_listing())
    })
    .bind(("127.0.0.1", 3000))?
    .run()
    .await?;

    cancel.cancel();

    handle.await.unwrap();

    Ok(())
}
