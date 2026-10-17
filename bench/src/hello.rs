fn main(){ if let Some(s)=std::env::args().nth(1){ std::thread::sleep(std::time::Duration::from_secs(s.parse().unwrap())); } }
