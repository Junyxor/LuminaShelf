use lumina_core::{provider::BookProvider, GutendexProvider, SearchQuery};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpListener,
};
use url::Url;

#[tokio::test]
async fn ui_page_sizes_do_not_omit_books_from_gutendex_fixed_pages() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = Url::parse(&format!("http://{}/books/", listener.local_addr().unwrap())).unwrap();
    let server = tokio::spawn(async move {
        while let Ok((mut socket, _)) = listener.accept().await {
            tokio::spawn(async move {
                let mut bytes = Vec::new();
                while !bytes.ends_with(b"\r\n\r\n") {
                    let mut byte = [0];
                    if socket.read_exact(&mut byte).await.is_err() {
                        return;
                    }
                    bytes.push(byte[0]);
                }
                let request = String::from_utf8(bytes).unwrap();
                let path = request
                    .lines()
                    .next()
                    .unwrap()
                    .split_whitespace()
                    .nth(1)
                    .unwrap();
                let request_url = Url::parse(&format!("http://localhost{path}")).unwrap();
                let page = request_url
                    .query_pairs()
                    .find(|(key, _)| key == "page")
                    .unwrap()
                    .1
                    .parse::<u64>()
                    .unwrap();
                let start = (page - 1) * 32 + 1;
                let end = (page * 32).min(70);
                let results = (start..=end).map(|id| serde_json::json!({ "id": id, "title": format!("Book {id}"), "formats": { "text/plain": "https://example.test/book.txt" } })).collect::<Vec<_>>();
                let body = serde_json::to_vec(&serde_json::json!({ "count": 70, "next": if end < 70 { Some("next") } else { None }, "results": results })).unwrap();
                let head = format!("HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", body.len());
                socket.write_all(head.as_bytes()).await.unwrap();
                socket.write_all(&body).await.unwrap();
            });
        }
    });
    let provider =
        GutendexProvider::with_client(reqwest::Client::builder().no_proxy().build().unwrap(), url)
            .unwrap();
    for size in [12, 24, 50] {
        let mut ids = Vec::new();
        for page in 1..=6 {
            let response = provider
                .search(SearchQuery {
                    text: "Book".into(),
                    page,
                    page_size: size,
                    formats: Vec::new(),
                })
                .await
                .unwrap();
            ids.extend(
                response
                    .items
                    .into_iter()
                    .map(|book| book.id.parse::<u64>().unwrap()),
            );
            if !response.has_next {
                break;
            }
        }
        assert_eq!(
            ids,
            (1..=70).collect::<Vec<_>>(),
            "all books must remain reachable at page size {size}"
        );
    }
    server.abort();
}
