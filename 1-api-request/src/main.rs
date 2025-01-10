use serde::Deserialize;

#[derive(Debug, Deserialize)]
#[allow(non_snake_case)]
struct Post {
    id: i32,
    title: String,
    body: String,
    userId: i32,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Create a reqwest client
    let client = reqwest::Client::new();

    // Fetch posts from JSONPlaceholder
    let posts: Vec<Post> = client
        .get("https://jsonplaceholder.typicode.com/posts")
        .send()
        .await?
        .json()
        .await?;

    // Print the first post
    if let Some(first_post) = posts.first() {
        println!("User ID: {}", first_post.userId);
        println!("Post ID: {}", first_post.id);
        println!("Title: {}", first_post.title);
        println!("Body: {}", first_post.body);
    }

    Ok(())
}
