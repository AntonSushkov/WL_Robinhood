extern crate rand;
extern crate humantime;

use std::collections::HashMap;
use std::fmt;
use std::sync::Arc;
use tokio::time::sleep;
use reqwest::{Client, Proxy};
use std::time::{Duration, SystemTime};
use reqwest::header::{HeaderMap, HeaderName, HeaderValue};
use serde_json::{json, Value};
use rand::seq::SliceRandom;
use rand::Rng;
use tokio::sync::Semaphore;
use std::fs;
use log::{error, info};

// Constants
const IP_CHECK_URL: &str = "https://ip.beget.ru/";
const MAX_RETRIES: u32 = 30;

#[derive(Debug)]
enum MyError {
    Reqwest(reqwest::Error),
    ErrorStr(String),
}
impl From<reqwest::Error> for MyError {
    fn from(err: reqwest::Error) -> Self {
        MyError::Reqwest(err)
    }
}
impl fmt::Display for MyError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            MyError::Reqwest(err) => write!(f, "Reqwest error: {}", err),
            MyError::ErrorStr(err) => write!(f, "Reqwest error: {}", err),
        }
    }
}
impl std::error::Error for MyError {}

async fn build_client(ip: &str, port: &str, login: &str, pass: &str) -> Result<Client, MyError> {
    let proxy = Proxy::https(format!("http://{}:{}", ip, port))?
        .basic_auth(login, pass);
    let client = Client::builder()
        .proxy(proxy)
        .timeout(Duration::from_secs(30))
        .build()?;
    Ok(client)
}

fn generate_user_agent() -> String {
    let browsers = vec![
        ("Chrome", rand::thread_rng().gen_range(100..115)),
        ("Firefox", rand::thread_rng().gen_range(100..115)),
        ("Safari", rand::thread_rng().gen_range(10..15)),
        ("Opera", rand::thread_rng().gen_range(70..81)),
        ("Edge", rand::thread_rng().gen_range(80..91))
    ];

    let platforms = vec![
        "Windows NT 10.0",
        "Macintosh; Intel Mac OS X 10_14_6",
        "X11; Linux x86_64"
    ];

    let platform = platforms.choose(&mut rand::thread_rng()).unwrap();
    let (browser, version) = browsers.choose(&mut rand::thread_rng()).unwrap();

    let gecko = if *browser == "Firefox" {
        "Gecko/20100101"
    } else {
        ""
    };

    let chrome_version = rand::thread_rng().gen_range(80..101);
    let user_agent = format!(
        "Mozilla/5.0 ({}) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/{}.0.{} {} {}/{}",
        platform, chrome_version, rand::thread_rng().gen_range(0..1000), gecko, browser, version
    );

    user_agent
}

fn setup_logger() -> Result<(), fern::InitError> {
    if !fs::metadata("Logs").is_ok() {
        fs::create_dir_all("Logs")?;
    }

    fern::Dispatch::new()
        .format(|out, message, record| {
            out.finish(format_args!(
                "[{} {} {}] {}",
                humantime::format_rfc3339_seconds(SystemTime::now()),
                record.level(),
                record.target(),
                message
            ))
        })
        // Set a global filter to `Info` level.
        // This will allow through `Info`, `Warn`, `Error`, and higher levels.
        .level(log::LevelFilter::Info)
        .chain(fern::log_file("Logs/logs.log")?)
        // Set up a custom filter for `stdout` to allow only `Info` and `Error` levels.
        .chain(fern::Dispatch::new()
            .filter(|record| matches!(record.level(), log::Level::Info | log::Level::Error))
            .chain(std::io::stdout())
        )
        .apply()?;
    Ok(())
}

struct Robinhood {
    email: String,
    invite_code: String,
    session: Client,
    headers: HeaderMap,
    captcha: String,
    cap_key: String,
}

impl Robinhood {
    fn generate_headers() -> HeaderMap {
        let ua = generate_user_agent();
        // println!("ua: {}", ua);
        let headers = vec![
            ("authority", "bonfire.robinhood.com"),
            ("accept", "*/*"),
            ("accept-language", "ru-RU,ru;q=0.9,en-US;q=0.8,en;q=0.7"),
            ("content-type", "text/plain;charset=UTF-8"),
            ("origin", "https://robinhood.com"),
            ("referer", "https://robinhood.com/"),
            ("sec-ch-ua", ""),
            ("sec-ch-ua-mobile", "?0"),
            ("sec-ch-ua-platform", "\"Windows\""),
            ("sec-fetch-dest", "empty"),
            ("sec-fetch-mode", "cors"),
            ("sec-fetch-site", "same-site"),
            ("user-agent", &ua),
        ];
        headers.into_iter()
            .filter_map(|(k, v)| {
                let key = HeaderName::from_bytes(k.as_bytes()).ok()?;
                let value = HeaderValue::from_str(v).ok()?;
                Some((key, value))
            })
            .collect()
    }

    pub async fn new(client: Client, email: &String, invite_code: &String, cap_key: &String) -> Result<Self, MyError> {

        let headers = Robinhood::generate_headers();

        Ok(Self {
            email: email.to_string(),
            invite_code: invite_code.to_string(),
            session: client,
            headers,
            captcha: String::new(),
            cap_key: cap_key.to_string(),
        })
    }

     // This function checks if the proxy is working
    async fn is_proxy_working(&self) -> bool {
        let response = self.session.get(IP_CHECK_URL).send().await;
            match response {
                Ok(res) => {
                    let content = res.text().await.unwrap_or_else(|_| "Failed to read response".to_string());
                    let cleaned_content = content.replace(" ", "")
                                                        .replace("{n", "")
                                                        .replace("\n", "");
                    info!("| {} | Response from : {}", self.email.clone(), cleaned_content);
                    return true
                }
                Err(_) => false,
            }
    }

    async fn send_invite(&mut self) -> Result<(), MyError> {
        let url = "https://bonfire.robinhood.com/waitlist/web3_wallet/email/spot";

        self.captcha = self.captcha_solver().await?;

        let mut data = HashMap::new();
            data.insert("email".to_string(), self.email.clone());
            data.insert("token".to_string(), self.captcha.clone());
            data.insert("referred_by".to_string(), self.invite_code.clone());

            // println!("{:#?}", data);

        let first_response  = self.session
            .post(url)
            .headers(self.headers.clone())
            .json(&data)
            .send()
            .await?;

            // println!("{:#?}", first_response.text().await? );

        if !first_response .status().is_success() {
            return Err(MyError::ErrorStr("Failed to send invite".into()));
        }

        let json_response: Value = first_response.json().await?;

        let position = json_response["position"].as_i64().unwrap_or(0);
        let referral_code = json_response["referral_code"].as_str().unwrap_or("");

        info!("| {} | Position: {} | Referral Code: {}", self.email.clone(), position, referral_code);

        Ok(())
    }

    async fn captcha_solver(&mut self) -> Result<String, MyError> {
        // let cap_key = " ";
        let website_url = "https://robinhood.com/web3-wallet/";
        let website_key = "6LcNCM0fAAAAAJLML8tBF-AMvjkws6z4bfar9VFF";
        let payload = json!({
            "clientKey": self.cap_key,
            "task":
            {
                "type":"RecaptchaV2EnterpriseTaskProxyless",
                "websiteURL":website_url,
                "websiteKey":website_key,
            }
        });
        let response = self.session
            .post("https://api.capmonster.cloud/createTask")
            .json(&payload)
            .send()
            .await?;

        let response_data: Value = response.json().await?;
        // info!("| {} | Response_data: {}", self.email.clone(), response_data);
        info!("| {} | Captcha - Solve...", self.email.clone());
        if let Some(task_id) = response_data["taskId"].as_u64() {
            sleep(Duration::from_secs(5)).await;

            for _ in 0..MAX_RETRIES {
                let payload = json!({
                    "clientKey":self.cap_key,
                    "taskId": task_id
                });
                let response = self.session
                    .post("https://api.capmonster.cloud/getTaskResult/")
                    .json(&payload)
                    .send()
                    .await?;

                let task_result: Value = response.json().await?;
                if let Some(status) = task_result["status"].as_str() {
                    if status == "ready" {
                        // info!("| {} | Status ready: {}", self.email.clone(), task_result);
                        if let Some(g_recaptcha_response) = task_result["solution"]["gRecaptchaResponse"].as_str() {
                            info!("| {} | Captcha - Ok", self.email.clone());
                            return Ok(g_recaptcha_response.to_string());
                        }
                    } else if status == "processing" {
                        // println!("Status processing: {}", task_result);
                        sleep(Duration::from_secs(3)).await;
                        continue;
                    } else {
                        error!("| {} | Captcha - Error", self.email.clone());
                        break;
                    }
                }
            }
        }

        Err(MyError::ErrorStr("Failed to solve the captcha.".to_string()))
    }

}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Set up the logger
    setup_logger()?;

    // Read invite_code once since it's constant for all
    let invite_code = std::fs::read_to_string("FILEs/invite_code.txt")?.trim().to_string();

    let cap_key = std::fs::read_to_string("FILEs/capmonster.txt")?.trim().to_string();



    // Open files for proxies and email data
    let proxy_lines = std::fs::read_to_string("FILEs/proxy.txt")?;
    let email_data_lines = std::fs::read_to_string("FILEs/email.txt")?;

    let paired_data: Vec<_> = proxy_lines.lines().zip(email_data_lines.lines()).collect();

    let max_concurrent_tasks = 5;  // Adjust this to control the flow rate.

    let semaphore = Arc::new(Semaphore::new(max_concurrent_tasks));

    let delay = rand::thread_rng().gen_range(2..5); // second delay
    let delay_duration = Duration::from_secs(delay);

    let futures: Vec<_> = paired_data.into_iter().map(|(proxy_line, email_data_line)| {
        let email_data_line = email_data_line.to_owned();
        let proxy_parts: Vec<String> = proxy_line.split(":").map(|s| s.to_string()).collect();

        let (ip, port, login, pass) = (proxy_parts[0].clone(), proxy_parts[1].clone(), proxy_parts[2].clone(), proxy_parts[3].clone());

        let email_parts: Vec<&str> = email_data_line.split(":").collect();
        let email = email_parts[0].to_string();
        // let imap = email_parts[1].to_string();
        let invite_code_clone = invite_code.clone();
        let cap_key_clone = cap_key.clone();

        let sema_clone = semaphore.clone();

        tokio::spawn(async move {
            sleep(delay_duration).await;  // This will delay each thread by the specified duration

            // Acquire semaphore permit
            let _permit = sema_clone.acquire().await;

            let client = match build_client(&ip, &port, &login, &pass).await {
                Ok(c) => c,
                Err(e) => {
                    error!("| {} | Failed to build client: {}", email, e.to_string());
                    return;
                }
            };
            info!("Start email: {}", email);

            let mut robinhood = match Robinhood::new(client, &email, &invite_code_clone, &cap_key_clone).await {
                Ok(n) => n,
                Err(e) => {
                    error!("| {} | Failed to create Nansen instance: {}", email, e.to_string());
                    return;
                }
            };

            if !robinhood.is_proxy_working().await {
                error!("| {} | Proxy not working", email);
                return;
            }

            match robinhood.send_invite().await {
                Ok(_) => info!("| {} | Email confirm", email),
                Err(_e) => error!("| {} | Failed to send invite", email),
            }


        })
    }).collect();

    futures::future::join_all(futures).await;

    Ok(())
}