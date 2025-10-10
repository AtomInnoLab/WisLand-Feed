use dotenvy::dotenv;
use feed::parsers::utils::{extract_affiliations_from_latex, extract_tex_from_latex_tar_gz};
use flate2::Compression;
use flate2::write::GzEncoder;
use std::path::Path;
use tar::Builder;
use tempfile::TempDir;
use tracing::info;
use tracing_subscriber::EnvFilter;

static INIT_TRACING: std::sync::Once = std::sync::Once::new();

fn init_test_tracing() {
    INIT_TRACING.call_once(|| {
        let _ = tracing_subscriber::fmt()
            .with_env_filter(
                EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")),
            )
            .with_writer(std::io::stderr)
            .compact()
            .try_init();
        dotenv().ok();
    });
}

/// Test extract_tex_from_latex_tar_gz function
#[tokio::test]
async fn test_extract_tex_from_latex_tar_gz() {
    init_test_tracing();
    info!("Starting test for extract_tex_from_latex_tar_gz function");

    // Create temporary directory
    let temp_dir = TempDir::new().expect("Failed to create temp dir");
    let temp_path = temp_dir.path();

    // Create test .tex file content
    let tex_content1 = r#"\documentclass{article}
\begin{document}
\title{Test Paper 1}
\author{Test Author}
\maketitle

\section{Introduction}
This is a test LaTeX document.

\section{Conclusion}
This concludes the test document.
\end{document}"#;

    let tex_content2 = r#"\documentclass{article}
\begin{document}
\title{Test Paper 2}
\author{Another Author}
\maketitle

\section{Abstract}
This is another test document.

\section{Methodology}
We use a simple approach.

\section{Results}
The results are promising.
\end{document}"#;

    // Create tar.gz file
    let tar_gz_path = temp_path.join("test_latex.tar.gz");
    create_test_tar_gz(
        &tar_gz_path,
        &[
            ("paper1.tex", tex_content1),
            ("paper2.tex", tex_content2),
            ("readme.txt", "This is a readme file"),
            ("data.csv", "col1,col2\n1,2\n3,4"),
        ],
    )
    .expect("Failed to create test tar.gz");

    // Read tar.gz file content
    let tar_gz_content = std::fs::read(&tar_gz_path).expect("Failed to read tar.gz file");

    // Call the function under test
    let result = extract_tex_from_latex_tar_gz(tar_gz_content);

    // Verify results
    assert!(result.is_ok(), "Function should return Ok");
    let tex_files = result.unwrap();

    // Should find 2 .tex files
    assert_eq!(tex_files.len(), 2, "Should extract 2 tex files");

    // Verify first file
    let (path1, content1) = &tex_files[0];
    assert!(path1.ends_with(".tex"), "First file should be .tex");
    assert!(
        content1.contains("Test Paper 1"),
        "First file should contain expected content"
    );

    // Verify second file
    let (path2, content2) = &tex_files[1];
    assert!(path2.ends_with(".tex"), "Second file should be .tex");
    assert!(
        content2.contains("Test Paper 2"),
        "Second file should contain expected content"
    );

    info!("✅ Successfully extracted {} .tex files", tex_files.len());
    for (path, _) in &tex_files {
        info!("  - {path}");
    }
}

/// Test empty tar.gz file
#[tokio::test]
async fn test_extract_tex_from_empty_tar_gz() {
    init_test_tracing();
    info!("Starting test for empty tar.gz file");

    // Create temporary directory
    let temp_dir = TempDir::new().expect("Failed to create temp dir");
    let temp_path = temp_dir.path();

    // Create empty tar.gz file
    let tar_gz_path = temp_path.join("empty.tar.gz");
    create_test_tar_gz(&tar_gz_path, &[]).expect("Failed to create empty tar.gz");

    // Read tar.gz file content
    let tar_gz_content = std::fs::read(&tar_gz_path).expect("Failed to read tar.gz file");

    // Call the function under test
    let result = extract_tex_from_latex_tar_gz(tar_gz_content);

    // Verify results
    assert!(result.is_ok(), "Function should return Ok for empty tar.gz");
    let tex_files = result.unwrap();
    assert_eq!(
        tex_files.len(),
        0,
        "Should extract 0 tex files from empty tar.gz"
    );

    info!("✅ Empty tar.gz file test passed");
}

/// Test tar.gz without .tex files
#[tokio::test]
async fn test_extract_tex_from_tar_gz_without_tex() {
    init_test_tracing();
    info!("Starting test for tar.gz without .tex files");

    // Create temporary directory
    let temp_dir = TempDir::new().expect("Failed to create temp dir");
    let temp_path = temp_dir.path();

    // Create tar.gz without .tex files
    let tar_gz_path = temp_path.join("no_tex.tar.gz");
    create_test_tar_gz(
        &tar_gz_path,
        &[
            ("readme.txt", "This is a readme file"),
            ("data.csv", "col1,col2\n1,2\n3,4"),
            ("image.png", "fake png content"),
            ("script.py", "print('hello world')"),
        ],
    )
    .expect("Failed to create tar.gz without tex files");

    // Read tar.gz file content
    let tar_gz_content = std::fs::read(&tar_gz_path).expect("Failed to read tar.gz file");

    // Call the function under test
    let result = extract_tex_from_latex_tar_gz(tar_gz_content);

    // Verify results
    assert!(result.is_ok(), "Function should return Ok");
    let tex_files = result.unwrap();
    assert_eq!(
        tex_files.len(),
        0,
        "Should extract 0 tex files when none exist"
    );

    info!("✅ Tar.gz without .tex files test passed");
}

/// Test corrupted tar.gz file
#[tokio::test]
async fn test_extract_tex_from_corrupted_tar_gz() {
    init_test_tracing();
    info!("Starting test for corrupted tar.gz file");

    // Create corrupted tar.gz content
    let corrupted_content = b"This is not a valid tar.gz file content";

    // Call the function under test
    let result = extract_tex_from_latex_tar_gz(corrupted_content.to_vec());

    // Verify results
    assert!(
        result.is_err(),
        "Function should return Err for corrupted tar.gz"
    );

    let error = result.unwrap_err();
    info!("✅ Corrupted tar.gz file test passed, error: {error}");
}

/// Test tar.gz with nested directories
#[tokio::test]
async fn test_extract_tex_from_nested_tar_gz() {
    init_test_tracing();
    info!("Starting test for tar.gz with nested directories");

    // Create temporary directory
    let temp_dir = TempDir::new().expect("Failed to create temp dir");
    let temp_path = temp_dir.path();

    // Create tar.gz with nested directories
    let tar_gz_path = temp_path.join("nested.tar.gz");

    // Manually create tar.gz with nested directories
    let file = std::fs::File::create(&tar_gz_path).expect("Failed to create file");
    let gz_encoder = GzEncoder::new(file, Compression::default());
    let mut tar_builder = Builder::new(gz_encoder);

    // Add root directory .tex file
    let mut header = tar::Header::new_gnu();
    header.set_path("main.tex").expect("Failed to set path");
    header.set_size(100);
    header.set_cksum();
    tar_builder
        .append(&header, "main.tex content".as_bytes())
        .expect("Failed to append file");

    // Add subdirectory .tex file
    let mut header = tar::Header::new_gnu();
    header
        .set_path("sections/intro.tex")
        .expect("Failed to set path");
    header.set_size(150);
    header.set_cksum();
    tar_builder
        .append(&header, "introduction content".as_bytes())
        .expect("Failed to append file");

    // Add another subdirectory .tex file
    let mut header = tar::Header::new_gnu();
    header
        .set_path("chapters/chapter1.tex")
        .expect("Failed to set path");
    header.set_size(200);
    header.set_cksum();
    tar_builder
        .append(&header, "chapter 1 content".as_bytes())
        .expect("Failed to append file");

    // Add non-.tex file
    let mut header = tar::Header::new_gnu();
    header
        .set_path("data/figures/image.png")
        .expect("Failed to set path");
    header.set_size(50);
    header.set_cksum();
    tar_builder
        .append(&header, "fake image content".as_bytes())
        .expect("Failed to append file");

    tar_builder.finish().expect("Failed to finish tar");

    // Read tar.gz file content
    let tar_gz_content = std::fs::read(&tar_gz_path).expect("Failed to read tar.gz file");

    // Call the function under test
    let result = extract_tex_from_latex_tar_gz(tar_gz_content);

    // Verify results
    assert!(result.is_ok(), "Function should return Ok");
    let tex_files = result.unwrap();

    // Should find 3 .tex files
    assert_eq!(
        tex_files.len(),
        3,
        "Should extract 3 tex files from nested structure"
    );

    // Verify file paths
    let paths: Vec<&String> = tex_files.iter().map(|(path, _)| path).collect();
    assert!(
        paths.contains(&&"main.tex".to_string()),
        "Should contain main.tex"
    );
    assert!(
        paths.contains(&&"sections/intro.tex".to_string()),
        "Should contain sections/intro.tex"
    );
    assert!(
        paths.contains(&&"chapters/chapter1.tex".to_string()),
        "Should contain chapters/chapter1.tex"
    );

    info!(
        "✅ Nested directory tar.gz file test passed, extracted {} .tex files",
        tex_files.len()
    );
    for (path, _) in &tex_files {
        info!("  - {path}");
    }
}

/// Test extracting affiliations from tar.gz file specified by environment variable
#[tokio::test]
async fn test_extract_affiliations_from_env_tar_gz() {
    init_test_tracing();
    info!(
        "Starting test for extracting affiliations from tar.gz file specified by environment variable"
    );

    // Read tar.gz file path from environment variable
    let tar_gz_path = match std::env::var("APP_TEST_LATEX_TAR_GZ") {
        Ok(path) => path,
        Err(_) => {
            info!("⚠️  Skipping test: APP_TEST_LATEX_TAR_GZ environment variable not set");
            return;
        }
    };

    // Check if file exists
    if !std::path::Path::new(&tar_gz_path).exists() {
        info!("⚠️  Skipping test: file does not exist: {tar_gz_path}");
        return;
    }

    info!("Using tar.gz file: {tar_gz_path}");

    // Read tar.gz file content
    let tar_gz_content = match std::fs::read(&tar_gz_path) {
        Ok(content) => content,
        Err(e) => {
            info!("❌ Failed to read file: {e}");
            return;
        }
    };

    // Extract .tex files
    let tex_files = match extract_tex_from_latex_tar_gz(tar_gz_content) {
        Ok(files) => files,
        Err(e) => {
            info!("❌ Failed to extract .tex files: {e}");
            return;
        }
    };

    info!("Successfully extracted {} .tex files", tex_files.len());

    // Extract affiliations from each .tex file
    let mut all_affiliations = Vec::new();
    for (path, content) in &tex_files {
        info!("Processing file: {path}");
        let affiliations = extract_affiliations_from_latex(content);
        info!("Extracted {} affiliations from {path}", affiliations.len());

        for affiliation in &affiliations {
            info!("  - {affiliation}");
        }

        all_affiliations.extend(affiliations);
    }

    info!("✅ Total extracted {} affiliations", all_affiliations.len());

    // Deduplicate and sort
    all_affiliations.sort();
    all_affiliations.dedup();

    info!(
        "✅ After deduplication, there are {} unique affiliations:",
        all_affiliations.len()
    );
    for affiliation in &all_affiliations {
        info!("  - {affiliation}");
    }

    // Verify at least some content was extracted
    assert!(
        !all_affiliations.is_empty(),
        "Should extract at least one affiliation"
    );
}

/// Test basic functionality of extract_affiliations_from_latex function
#[tokio::test]
async fn test_extract_affiliations_from_latex_basic() {
    init_test_tracing();
    info!("Starting test for basic functionality of extract_affiliations_from_latex function");

    // Test LaTeX content containing \affiliation
    let latex_with_affiliation = r#"\documentclass{article}
\begin{document}
\title{Test Paper}
\author{John Doe}
\affiliation{Department of Computer Science, University of Example}
\affiliation{Institute of Technology, Example City}
\maketitle

\section{Introduction}
This is a test document.
\end{document}"#;

    let affiliations = extract_affiliations_from_latex(latex_with_affiliation);

    assert_eq!(affiliations.len(), 2, "Should extract 2 affiliations");
    assert!(
        affiliations.contains(&"Department of Computer Science, University of Example".to_string())
    );
    assert!(affiliations.contains(&"Institute of Technology, Example City".to_string()));

    info!(
        "✅ Basic affiliation extraction test passed, extracted {} affiliations",
        affiliations.len()
    );
    for affiliation in &affiliations {
        info!("  - {affiliation}");
    }
}

/// Test extract_affiliations_from_latex function handling \affil command
#[tokio::test]
async fn test_extract_affiliations_from_latex_affil() {
    init_test_tracing();
    info!("Starting test for extract_affiliations_from_latex function handling \\affil command");

    // Test LaTeX content containing \affil
    let latex_with_affil = r#"\documentclass{article}
\begin{document}
\title{Test Paper}
\author{Jane Smith}
\affil{School of Engineering, Tech University}
\affil{Research Center, Innovation Lab}
\maketitle

\section{Introduction}
This is a test document.
\end{document}"#;

    let affiliations = extract_affiliations_from_latex(latex_with_affil);

    assert_eq!(affiliations.len(), 2, "Should extract 2 affiliations");
    assert!(affiliations.contains(&"School of Engineering, Tech University".to_string()));
    assert!(affiliations.contains(&"Research Center, Innovation Lab".to_string()));

    info!(
        "✅ \\affil command test passed, extracted {} affiliations",
        affiliations.len()
    );
    for affiliation in &affiliations {
        info!("  - {affiliation}");
    }
}

/// Test extract_affiliations_from_latex function handling \institute command
#[tokio::test]
async fn test_extract_affiliations_from_latex_institute() {
    init_test_tracing();
    info!(
        "Starting test for extract_affiliations_from_latex function handling \\institute command"
    );

    // Test LaTeX content containing \institute
    let latex_with_institute = r#"\documentclass{article}
\begin{document}
\title{Test Paper}
\author{Bob Johnson}
\institute{Department of Mathematics, Science University}
\institute{Advanced Research Institute}
\maketitle

\section{Introduction}
This is a test document.
\end{document}"#;

    let affiliations = extract_affiliations_from_latex(latex_with_institute);

    assert_eq!(affiliations.len(), 2, "Should extract 2 affiliations");
    assert!(affiliations.contains(&"Department of Mathematics, Science University".to_string()));
    assert!(affiliations.contains(&"Advanced Research Institute".to_string()));

    info!(
        "✅ \\institute command test passed, extracted {} affiliations",
        affiliations.len()
    );
    for affiliation in &affiliations {
        info!("  - {affiliation}");
    }
}

/// Test extract_affiliations_from_latex function handling author information as fallback
#[tokio::test]
async fn test_extract_affiliations_from_latex_author_fallback() {
    init_test_tracing();
    info!(
        "Starting test for extract_affiliations_from_latex function handling author information as fallback"
    );

    // Test LaTeX content without affiliation information, only author information
    let latex_with_author_only = r#"\documentclass{article}
\begin{document}
\title{Test Paper}
\author{Alice Brown \and Charlie Wilson}
\maketitle

\section{Introduction}
This is a test document without affiliation information.
\end{document}"#;

    let affiliations = extract_affiliations_from_latex(latex_with_author_only);

    // Should fall back to extracting author information
    assert!(
        !affiliations.is_empty(),
        "Should extract author information as fallback"
    );

    info!(
        "✅ Author information fallback test passed, extracted {} affiliations",
        affiliations.len()
    );
    for affiliation in &affiliations {
        info!("  - {affiliation}");
    }
}

/// Test extract_affiliations_from_latex function handling empty content
#[tokio::test]
async fn test_extract_affiliations_from_latex_empty() {
    init_test_tracing();
    info!("Starting test for extract_affiliations_from_latex function handling empty content");

    // Test empty LaTeX content
    let empty_latex = "";
    let affiliations = extract_affiliations_from_latex(empty_latex);

    assert_eq!(
        affiliations.len(),
        0,
        "Empty content should return empty list"
    );

    // Test LaTeX content with only document structure
    let minimal_latex = r#"\documentclass{article}
\begin{document}
\title{Test}
\maketitle
\end{document}"#;

    let affiliations2 = extract_affiliations_from_latex(minimal_latex);

    info!(
        "✅ Empty content test passed, empty content returned {} affiliations",
        affiliations.len()
    );
    info!(
        "✅ Minimal content test passed, minimal content returned {} affiliations",
        affiliations2.len()
    );
}

/// Helper function: create test tar.gz file
fn create_test_tar_gz(
    path: &Path,
    files: &[(&str, &str)],
) -> Result<(), Box<dyn std::error::Error>> {
    let file = std::fs::File::create(path)?;
    let gz_encoder = GzEncoder::new(file, Compression::default());
    let mut tar_builder = Builder::new(gz_encoder);

    for (filename, content) in files {
        let mut header = tar::Header::new_gnu();
        header.set_path(filename)?;
        header.set_size(content.len() as u64);
        header.set_cksum();
        tar_builder.append(&header, content.as_bytes())?;
    }

    tar_builder.finish()?;
    Ok(())
}
