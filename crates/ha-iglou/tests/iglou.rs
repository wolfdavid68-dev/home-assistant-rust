//! Iglou pour de vrai, contre un faux Firebase (sur ce PC) qui applique les
//! mêmes règles que l'appli : un compte ne lit et n'écrit que les maisons dont
//! il est membre, et le jeton expire.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    reason = "dans un test, un échec doit simplement faire échouer le test"
)]

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use axum::body::Bytes;
use axum::http::{HeaderMap, Method, StatusCode, Uri};
use axum::response::{IntoResponse, Response};
use ha_core::{Core, EntityId, Origin};
use ha_iglou::{Client, Endpoints, Iglou, IglouError};
use serde_json::{Map, Value, json};
use zeroize::Zeroizing;

const PREFIX: &str = "/v1/projects/projet-test/databases/(default)/documents";
const KEY: &str = "AIzaCleDeTestQuiSuffitBien_123";

/// La base du faux Firebase : chemin du document → champs.
#[derive(Default)]
struct Fake {
    docs: BTreeMap<String, Map<String, Value>>,
    /// Le jeton valable (un autre est refusé en 401).
    token: String,
    sign_ins: usize,
    commits: Vec<Value>,
}

type Shared = Arc<Mutex<Fake>>;

fn fields(value: Value) -> Map<String, Value> {
    let Value::Object(map) = value else { panic!() };
    map
}

fn s(text: &str) -> Value {
    json!({ "stringValue": text })
}

fn n(value: i64) -> Value {
    json!({ "integerValue": value.to_string() })
}

fn seed() -> Fake {
    let mut fake = Fake::default();
    let mut put = |path: &str, value: Value| {
        fake.docs.insert(path.to_owned(), fields(value));
    };
    put(
        "maisons/m1",
        json!({ "nom": s("Chez nous"), "codeInvitation": s("ABC123") }),
    );
    put("invitations/ABC123", json!({ "maisonId": s("m1") }));
    put("maisons/m2", json!({ "nom": s("Autre maison") }));
    put(
        "maisons/m1/appareils/frigo",
        json!({ "nom": s("Frigo"), "type": s("frigo"), "ordre": n(0) }),
    );
    put(
        "maisons/m1/aliments/lait",
        json!({
            "nom": s("Lait"), "quantite": n(0), "quantiteMax": n(2), "unite": s("L"),
            "dansPanier": json!({"booleanValue": false}), "appareilId": s("frigo"),
            "ajoutePar": s("david"), "modifiePar": s("david"),
        }),
    );
    put(
        "maisons/m1/aliments/oeufs",
        json!({
            "nom": s("Œufs"), "quantite": n(6), "quantiteMax": n(6), "unite": s("pièce"),
            "dansPanier": json!({"booleanValue": false}), "appareilId": s("frigo"),
        }),
    );
    put(
        "maisons/m2/aliments/secret",
        json!({ "nom": s("Secret"), "quantite": n(0), "quantiteMax": n(1) }),
    );
    fake
}

/// Membre de la maison ?
fn member(fake: &Fake, maison: &str) -> bool {
    fake.docs
        .contains_key(&format!("maisons/{maison}/membres/maison-uid"))
}

fn maison_of(path: &str) -> Option<&str> {
    let mut parts = path.split('/');
    (parts.next()? == "maisons").then(|| parts.next()).flatten()
}

fn allowed(fake: &Fake, path: &str) -> bool {
    match maison_of(path) {
        Some(maison) => member(fake, maison) || path.contains("/membres/"),
        None => path.starts_with("invitations/"),
    }
}

fn document(path: &str, fields: &Map<String, Value>) -> Value {
    json!({
        "name": format!("projects/projet-test/databases/(default)/documents/{path}"),
        "fields": fields,
    })
}

async fn handler(
    axum::extract::State(fake): axum::extract::State<Shared>,
    method: Method,
    uri: Uri,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    let path = uri.path().to_owned();
    let mut fake = fake.lock().unwrap();
    if path == "/v1/accounts:signInWithPassword" {
        assert_eq!(uri.query(), Some(format!("key={KEY}").as_str()));
        let body: Value = serde_json::from_slice(&body).unwrap();
        if body["email"] != "maison@exemple.fr" || body["password"] != "bon mot de passe" {
            return (
                StatusCode::BAD_REQUEST,
                axum::Json(json!({"error": {"message": "INVALID_LOGIN_CREDENTIALS"}})),
            )
                .into_response();
        }
        fake.sign_ins += 1;
        fake.token = format!("jeton-{}", fake.sign_ins);
        return axum::Json(
            json!({ "idToken": fake.token, "localId": "maison-uid", "expiresIn": "3600" }),
        )
        .into_response();
    }
    let bearer = headers
        .get("authorization")
        .and_then(|v| v.to_str().ok())
        .unwrap_or_default();
    if bearer != format!("Bearer {}", fake.token) {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    let Some(rest) = path.strip_prefix(PREFIX) else {
        return StatusCode::NOT_FOUND.into_response();
    };
    match (method, rest) {
        (Method::POST, ":runQuery") => {
            let rows: Vec<Value> = fake
                .docs
                .iter()
                .filter(|(p, f)| p.contains("/membres/") && f.get("uid") == Some(&s("maison-uid")))
                .map(|(p, f)| json!({ "document": document(p, f) }))
                .collect();
            axum::Json(Value::Array(rows)).into_response()
        }
        (Method::POST, ":commit") => {
            let body: Value = serde_json::from_slice(&body).unwrap();
            for write in body["writes"].as_array().unwrap() {
                let name = write["update"]["name"]
                    .as_str()
                    .or_else(|| write["delete"].as_str())
                    .unwrap();
                let doc = name.split("/documents/").nth(1).unwrap().to_owned();
                if !allowed(&fake, &doc) {
                    return StatusCode::FORBIDDEN.into_response();
                }
                // Rejoindre : le code doit être le bon.
                if doc.contains("/membres/") {
                    let maison = maison_of(&doc).unwrap();
                    let code = fake.docs[&format!("maisons/{maison}")]
                        .get("codeInvitation")
                        .cloned();
                    if write["update"]["fields"]["code"] != code.unwrap_or(Value::Null) {
                        return StatusCode::FORBIDDEN.into_response();
                    }
                }
                if write.get("delete").is_some() {
                    fake.docs.remove(&doc);
                    continue;
                }
                let exists = fake.docs.contains_key(&doc);
                if let Some(expected) = write
                    .pointer("/currentDocument/exists")
                    .and_then(Value::as_bool)
                    && expected != exists
                {
                    return StatusCode::BAD_REQUEST.into_response();
                }
                let new = write["update"]["fields"].as_object().unwrap().clone();
                let entry = fake.docs.entry(doc).or_default();
                match write
                    .pointer("/updateMask/fieldPaths")
                    .and_then(Value::as_array)
                {
                    Some(mask) => {
                        for field in mask {
                            let field = field.as_str().unwrap();
                            entry.insert(field.to_owned(), new[field].clone());
                        }
                    }
                    None => *entry = new,
                }
                for transform in write["updateTransforms"].as_array().into_iter().flatten() {
                    entry.insert(
                        transform["fieldPath"].as_str().unwrap().to_owned(),
                        json!({"timestampValue": "2026-10-21T20:00:00Z"}),
                    );
                }
            }
            fake.commits.push(body);
            axum::Json(json!({ "writeResults": [] })).into_response()
        }
        (Method::GET, rest) => {
            let doc = rest.trim_start_matches('/');
            if !allowed(&fake, doc) {
                return StatusCode::FORBIDDEN.into_response();
            }
            if doc.split('/').count() % 2 == 1 {
                // Une collection : ses documents directs, deux par page.
                let all: Vec<Value> = fake
                    .docs
                    .iter()
                    .filter(|(p, _)| {
                        p.starts_with(&format!("{doc}/"))
                            && p.split('/').count() == doc.split('/').count() + 1
                    })
                    .map(|(p, f)| document(p, f))
                    .collect();
                let start: usize = uri
                    .query()
                    .and_then(|q| q.split('&').find_map(|kv| kv.strip_prefix("pageToken=")))
                    .and_then(|t| t.parse().ok())
                    .unwrap_or(0);
                let page: Vec<Value> = all.iter().skip(start).take(2).cloned().collect();
                let mut out = json!({ "documents": page });
                if start + 2 < all.len() {
                    out["nextPageToken"] = json!((start + 2).to_string());
                }
                axum::Json(out).into_response()
            } else {
                match fake.docs.get(doc) {
                    Some(f) => axum::Json(document(doc, f)).into_response(),
                    None => StatusCode::NOT_FOUND.into_response(),
                }
            }
        }
        _ => StatusCode::NOT_FOUND.into_response(),
    }
}

async fn serve(fake: Fake) -> (String, Shared) {
    let shared: Shared = Arc::new(Mutex::new(fake));
    let app = axum::Router::new()
        .fallback(handler)
        .with_state(Arc::clone(&shared));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    (format!("http://{address}"), shared)
}

fn client(base: &str, password: &str) -> Client {
    Client::new(
        ha_tls::web_client(false).unwrap(),
        Endpoints::local(base),
        "projet-test",
        KEY,
        "maison@exemple.fr",
        Zeroizing::new(password.to_owned()),
    )
    .unwrap()
}

#[tokio::test]
async fn joins_reads_and_writes_the_list() {
    let (base, fake) = serve(seed()).await;
    let client = client(&base, "bon mot de passe");
    assert!(
        client.maisons().await.unwrap().is_empty(),
        "membre de rien au départ"
    );
    // Sans être membre, la liste est refusée.
    assert_eq!(
        client.shopping_list("m1").await.unwrap_err(),
        IglouError::Denied
    );
    // Un mauvais code ne fait rien rejoindre.
    assert!(client.join("ZZZ999", "Maison").await.is_err());
    let maison = client.join("abc 123", "Maison").await.unwrap();
    assert_eq!(maison.nom, "Chez nous");
    assert_eq!(
        client.maisons().await.unwrap(),
        std::slice::from_ref(&maison)
    );
    // Rejoindre une deuxième fois ne recrée rien.
    client.join("ABC123", "Maison").await.unwrap();

    let items = client.shopping_list("m1").await.unwrap();
    assert_eq!(items.len(), 1, "les œufs sont pleins");
    assert_eq!(
        (items[0].summary.as_str(), items[0].quantity.as_str()),
        ("Lait", "2 L")
    );

    client.add("m1", "  Pain\u{7} ").await.unwrap();
    client.add("m1", "Beurre").await.unwrap();
    client.add("m1", "Café").await.unwrap();
    let items = client.shopping_list("m1").await.unwrap();
    let names: Vec<_> = items.iter().map(|i| i.summary.as_str()).collect();
    // Lu sur plusieurs pages ; les articles libres après les aliments.
    assert_eq!(names, ["Lait", "Beurre", "Café", "Pain"]);
    let pain = items
        .iter()
        .find(|i| i.summary == "Pain")
        .unwrap()
        .uid
        .clone();

    client.set_in_cart("m1", "a:lait", true).await.unwrap();
    client.set_in_cart("m1", &pain, true).await.unwrap();
    {
        let fake = fake.lock().unwrap();
        let lait = &fake.docs["maisons/m1/aliments/lait"];
        assert_eq!(lait["dansPanier"], json!({"booleanValue": true}));
        assert_eq!(
            lait["modifiePar"],
            s("maison-uid"),
            "les règles de l'appli le veulent"
        );
        assert_eq!(
            lait["quantite"],
            n(0),
            "la maison ne touche jamais aux quantités"
        );
        assert!(lait.contains_key("updatedAt"));
        let added = fake
            .docs
            .iter()
            .find(|(_, f)| f.get("nom") == Some(&s("Pain")))
            .unwrap()
            .1;
        assert_eq!(added["ajoutePar"], s("maison-uid"));
        assert_eq!(added["unite"], s("pièce"));
        assert!(added.contains_key("createdAt"));
    }
    client.rename("m1", &pain, "Baguette").await.unwrap();
    assert!(
        client.rename("m1", "a:lait", "Lait entier").await.is_err(),
        "un aliment se renomme dans l'appli"
    );
    assert!(
        client.remove("m1", "a:lait").await.is_err(),
        "un aliment se range dans l'appli"
    );
    client.remove("m1", &pain).await.unwrap();
    let names: Vec<_> = client
        .shopping_list("m1")
        .await
        .unwrap()
        .into_iter()
        .map(|i| i.summary)
        .collect();
    assert_eq!(names, ["Lait", "Beurre", "Café"]);

    // Jamais la maison d'à côté.
    assert_eq!(
        client.shopping_list("m2").await.unwrap_err(),
        IglouError::Denied
    );
    assert!(client.shopping_list("../m2").await.is_err());
}

#[tokio::test]
async fn a_refused_token_is_renewed_once() {
    let mut fake = seed();
    fake.docs.insert(
        "maisons/m1/membres/maison-uid".into(),
        fields(json!({ "uid": s("maison-uid"), "maisonId": s("m1") })),
    );
    let (base, fake) = serve(fake).await;
    let client = client(&base, "bon mot de passe");
    client.shopping_list("m1").await.unwrap();
    // Le jeton expire côté serveur : une seule reconnexion, et ça repart.
    fake.lock().unwrap().token = "un autre".into();
    client.shopping_list("m1").await.unwrap();
    assert_eq!(fake.lock().unwrap().sign_ins, 2);
}

#[tokio::test]
async fn a_wrong_password_is_lasting() {
    let (base, _) = serve(seed()).await;
    let error = client(&base, "faux").maisons().await.unwrap_err();
    assert_eq!(error, IglouError::Credentials);
    assert!(error.is_lasting());
}

#[tokio::test]
async fn the_shopping_list_entity() {
    let mut fake = seed();
    fake.docs.insert(
        "maisons/m1/membres/maison-uid".into(),
        fields(json!({ "uid": s("maison-uid"), "maisonId": s("m1") })),
    );
    let (base, fake) = serve(fake).await;
    let core = Arc::new(Core::new());
    let iglou = Iglou::start(
        client(&base, "bon mot de passe"),
        "m1".into(),
        Arc::clone(&core),
    );
    let id: EntityId = ha_iglou::ENTITY.parse().unwrap();
    let wait_for = |want: &'static str| {
        let core = Arc::clone(&core);
        let id = id.clone();
        async move {
            for _ in 0..300 {
                if core.states.get(&id).is_some_and(|s| s.state() == want) {
                    return core.states.get(&id).unwrap();
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
            panic!("{} n'est jamais passé à {want}", id.as_str());
        }
    };
    let state = wait_for("1").await;
    assert_eq!(state.attributes()["friendly_name"], "Liste de courses");
    assert_eq!(state.attributes()["items"][0]["summary"], "Lait");

    let call = |service: &str, data: Value| {
        let Value::Object(data) = data else { panic!() };
        core.call_service_tracked(
            Origin::User("david".into()),
            "todo",
            service,
            id.clone(),
            data,
        )
        .unwrap()
        .1
    };
    // Ajouter : l'ordre attend la réponse d'Iglou, puis la liste est relue.
    call("add_item", json!({ "item": "Pain" }))
        .await
        .unwrap()
        .unwrap();
    wait_for("2").await;
    // Une deuxième fois (« pain ») : pas de doublon.
    call("add_item", json!({ "item": "pain" }))
        .await
        .unwrap()
        .unwrap();
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert_eq!(core.states.get(&id).unwrap().state(), "2");
    // Cocher par le nom.
    call(
        "update_item",
        json!({ "item": "lait", "status": "completed" }),
    )
    .await
    .unwrap()
    .unwrap();
    let state = wait_for("1").await;
    assert_eq!(state.attributes()["items"][0]["status"], "completed");
    // Ranger se fait dans l'appli.
    let refused = call("remove_completed_items", json!({})).await.unwrap();
    assert!(refused.unwrap_err().contains("appli Iglou"));
    assert_eq!(
        fake.lock().unwrap().docs.len(),
        seed().docs.len() + 2,
        "rien d'effacé"
    );
    iglou.stop().await;
}
