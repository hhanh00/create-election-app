use std::fs::File;

use anyhow::Error;
use bip0039::Mnemonic;
use orchard::keys::{FullViewingKey, Scope, SpendingKey};
use r2d2::Pool;
use r2d2_sqlite::SqliteConnectionManager;
use serde::{Deserialize, Serialize};
use slug::slugify;
use tauri::ipc::Channel;
use zcash_vote::{
    address::VoteAddress,
    db::create_schema,
    download::download_reference_data,
    trees::{compute_cmx_root, compute_nf_root},
    CandidateChoice, Election,
};

#[derive(Clone, Serialize, Deserialize, Debug)]
pub struct ElectionTemplate {
    name: String,
    start: u32,
    end: u32,
    question: String,
    choices: String,
    signature_required: bool,
}

#[derive(Clone, Serialize, Deserialize, Debug)]
pub struct ElectionData {
    pub seed: String,
    pub election: Election,
}

#[tauri::command]
async fn create_election(
    election: ElectionTemplate,
    channel: Channel<u32>,
) -> Result<String, String> {
    let e = async {
        println!("{}", serde_json::to_string(&election).unwrap());
        let mnemonic = Mnemonic::generate(bip0039::Count::Words24);
        let phrase = mnemonic.phrase().to_string();
        let seed = mnemonic.to_seed("vote");
        let candidates = election
            .choices
            .trim()
            .split("\n")
            .enumerate()
            .map(|(i, choice)| {
                let spk = SpendingKey::from_zip32_seed(&seed, 159, i as u32).unwrap();
                let fvk = FullViewingKey::from(&spk);
                let address = fvk.address_at(0u64, Scope::External);
                let vote_address = VoteAddress(address);

                CandidateChoice {
                    address: vote_address.to_string(),
                    choice: choice.to_string(),
                }
            })
            .collect::<Vec<_>>();
        let id = slugify(&election.name);

        let manager = SqliteConnectionManager::memory();
        let pool = Pool::new(manager)?;

        let start = election.start;
        let end = election.end;

        let mut e = Election {
            id,
            name: election.name,
            start_height: start,
            end_height: end,
            question: election.question,
            candidates,
            signature_required: election.signature_required,
            cmx: [0u8; 32],
            nf: [0u8; 32],
        };

        let connection = pool.get()?;
        create_schema(&connection)?;

        let lwd_url = std::env::var("LWD_URL").unwrap_or("https://zec.rocks".to_string());
        let ch = channel.clone();
        let (connection, _) = download_reference_data(connection, &e, None, &lwd_url, move |h| {
            let p = (100 * (h - start)) / (end - start) / 2;
            let _ = ch.send(p);
        })
        .await?;
        println!("downloaded");

        let nf_root = compute_nf_root(&connection)?;
        channel.send(75)?;
        println!("nf_root");
        let cmx_root = compute_cmx_root(&connection)?;
        channel.send(100)?;
        println!("cmx_root");

        e.nf.copy_from_slice(&nf_root);
        e.cmx.copy_from_slice(&cmx_root);

        let e = ElectionData {
            seed: phrase,
            election: e,
        };

        let e = serde_json::to_string(&e)?;

        Ok::<_, Error>(e)
    };
    e.await.map_err(|e| e.to_string())
}

#[tauri::command]
fn save_election(path: String, election: Election) -> Result<(), String> {
    let r = || {
        let mut f = File::create(path)?;
        serde_json::to_writer(&mut f, &election)?;
        Ok::<_, Error>(())
    };
    r().map_err(|e| e.to_string())
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .invoke_handler(tauri::generate_handler![create_election, save_election])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}