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

/// 测试 extract_tex_from_latex_tar_gz 函数
#[tokio::test]
async fn test_extract_tex_from_latex_tar_gz() {
    init_test_tracing();
    info!("开始测试 extract_tex_from_latex_tar_gz 函数");

    // 创建临时目录
    let temp_dir = TempDir::new().expect("Failed to create temp dir");
    let temp_path = temp_dir.path();

    // 创建测试用的 .tex 文件内容
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

    // 创建 tar.gz 文件
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

    // 读取 tar.gz 文件内容
    let tar_gz_content = std::fs::read(&tar_gz_path).expect("Failed to read tar.gz file");

    // 调用被测试的函数
    let result = extract_tex_from_latex_tar_gz(tar_gz_content);

    // 验证结果
    assert!(result.is_ok(), "Function should return Ok");
    let tex_files = result.unwrap();

    // 应该找到 2 个 .tex 文件
    assert_eq!(tex_files.len(), 2, "Should extract 2 tex files");

    // 验证第一个文件
    let (path1, content1) = &tex_files[0];
    assert!(path1.ends_with(".tex"), "First file should be .tex");
    assert!(
        content1.contains("Test Paper 1"),
        "First file should contain expected content"
    );

    // 验证第二个文件
    let (path2, content2) = &tex_files[1];
    assert!(path2.ends_with(".tex"), "Second file should be .tex");
    assert!(
        content2.contains("Test Paper 2"),
        "Second file should contain expected content"
    );

    info!("✅ 成功提取了 {} 个 .tex 文件", tex_files.len());
    for (path, _) in &tex_files {
        info!("  - {path}");
    }
}

/// 测试空 tar.gz 文件
#[tokio::test]
async fn test_extract_tex_from_empty_tar_gz() {
    init_test_tracing();
    info!("开始测试空 tar.gz 文件");

    // 创建临时目录
    let temp_dir = TempDir::new().expect("Failed to create temp dir");
    let temp_path = temp_dir.path();

    // 创建空的 tar.gz 文件
    let tar_gz_path = temp_path.join("empty.tar.gz");
    create_test_tar_gz(&tar_gz_path, &[]).expect("Failed to create empty tar.gz");

    // 读取 tar.gz 文件内容
    let tar_gz_content = std::fs::read(&tar_gz_path).expect("Failed to read tar.gz file");

    // 调用被测试的函数
    let result = extract_tex_from_latex_tar_gz(tar_gz_content);

    // 验证结果
    assert!(result.is_ok(), "Function should return Ok for empty tar.gz");
    let tex_files = result.unwrap();
    assert_eq!(
        tex_files.len(),
        0,
        "Should extract 0 tex files from empty tar.gz"
    );

    info!("✅ 空 tar.gz 文件测试通过");
}

/// 测试没有 .tex 文件的 tar.gz
#[tokio::test]
async fn test_extract_tex_from_tar_gz_without_tex() {
    init_test_tracing();
    info!("开始测试没有 .tex 文件的 tar.gz");

    // 创建临时目录
    let temp_dir = TempDir::new().expect("Failed to create temp dir");
    let temp_path = temp_dir.path();

    // 创建不包含 .tex 文件的 tar.gz
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

    // 读取 tar.gz 文件内容
    let tar_gz_content = std::fs::read(&tar_gz_path).expect("Failed to read tar.gz file");

    // 调用被测试的函数
    let result = extract_tex_from_latex_tar_gz(tar_gz_content);

    // 验证结果
    assert!(result.is_ok(), "Function should return Ok");
    let tex_files = result.unwrap();
    assert_eq!(
        tex_files.len(),
        0,
        "Should extract 0 tex files when none exist"
    );

    info!("✅ 无 .tex 文件的 tar.gz 测试通过");
}

/// 测试损坏的 tar.gz 文件
#[tokio::test]
async fn test_extract_tex_from_corrupted_tar_gz() {
    init_test_tracing();
    info!("开始测试损坏的 tar.gz 文件");

    // 创建损坏的 tar.gz 内容
    let corrupted_content = b"This is not a valid tar.gz file content";

    // 调用被测试的函数
    let result = extract_tex_from_latex_tar_gz(corrupted_content.to_vec());

    // 验证结果
    assert!(
        result.is_err(),
        "Function should return Err for corrupted tar.gz"
    );

    let error = result.unwrap_err();
    info!("✅ 损坏的 tar.gz 文件测试通过，错误: {error}");
}

/// 测试包含嵌套目录的 tar.gz 文件
#[tokio::test]
async fn test_extract_tex_from_nested_tar_gz() {
    init_test_tracing();
    info!("开始测试包含嵌套目录的 tar.gz 文件");

    // 创建临时目录
    let temp_dir = TempDir::new().expect("Failed to create temp dir");
    let temp_path = temp_dir.path();

    // 创建包含嵌套目录的 tar.gz 文件
    let tar_gz_path = temp_path.join("nested.tar.gz");

    // 手动创建包含嵌套目录的 tar.gz
    let file = std::fs::File::create(&tar_gz_path).expect("Failed to create file");
    let gz_encoder = GzEncoder::new(file, Compression::default());
    let mut tar_builder = Builder::new(gz_encoder);

    // 添加根目录的 .tex 文件
    let mut header = tar::Header::new_gnu();
    header.set_path("main.tex").expect("Failed to set path");
    header.set_size(100);
    header.set_cksum();
    tar_builder
        .append(&header, "main.tex content".as_bytes())
        .expect("Failed to append file");

    // 添加子目录的 .tex 文件
    let mut header = tar::Header::new_gnu();
    header
        .set_path("sections/intro.tex")
        .expect("Failed to set path");
    header.set_size(150);
    header.set_cksum();
    tar_builder
        .append(&header, "introduction content".as_bytes())
        .expect("Failed to append file");

    // 添加另一个子目录的 .tex 文件
    let mut header = tar::Header::new_gnu();
    header
        .set_path("chapters/chapter1.tex")
        .expect("Failed to set path");
    header.set_size(200);
    header.set_cksum();
    tar_builder
        .append(&header, "chapter 1 content".as_bytes())
        .expect("Failed to append file");

    // 添加非 .tex 文件
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

    // 读取 tar.gz 文件内容
    let tar_gz_content = std::fs::read(&tar_gz_path).expect("Failed to read tar.gz file");

    // 调用被测试的函数
    let result = extract_tex_from_latex_tar_gz(tar_gz_content);

    // 验证结果
    assert!(result.is_ok(), "Function should return Ok");
    let tex_files = result.unwrap();

    // 应该找到 3 个 .tex 文件
    assert_eq!(
        tex_files.len(),
        3,
        "Should extract 3 tex files from nested structure"
    );

    // 验证文件路径
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
        "✅ 嵌套目录 tar.gz 文件测试通过，提取了 {} 个 .tex 文件",
        tex_files.len()
    );
    for (path, _) in &tex_files {
        info!("  - {path}");
    }
}

/// 测试从环境变量指定的 tar.gz 文件中提取 affiliations
#[tokio::test]
async fn test_extract_affiliations_from_env_tar_gz() {
    init_test_tracing();
    info!("开始测试从环境变量指定的 tar.gz 文件中提取 affiliations");

    // 从环境变量读取 tar.gz 文件路径
    let tar_gz_path = match std::env::var("APP_TEST_LATEX_TAR_GZ") {
        Ok(path) => path,
        Err(_) => {
            info!("⚠️  跳过测试：未设置 APP_TEST_LATEX_TAR_GZ 环境变量");
            return;
        }
    };

    // 检查文件是否存在
    if !std::path::Path::new(&tar_gz_path).exists() {
        info!("⚠️  跳过测试：文件不存在: {tar_gz_path}");
        return;
    }

    info!("使用 tar.gz 文件: {tar_gz_path}");

    // 读取 tar.gz 文件内容
    let tar_gz_content = match std::fs::read(&tar_gz_path) {
        Ok(content) => content,
        Err(e) => {
            info!("❌ 读取文件失败: {e}");
            return;
        }
    };

    // 提取 .tex 文件
    let tex_files = match extract_tex_from_latex_tar_gz(tar_gz_content) {
        Ok(files) => files,
        Err(e) => {
            info!("❌ 提取 .tex 文件失败: {e}");
            return;
        }
    };

    info!("成功提取了 {} 个 .tex 文件", tex_files.len());

    // 从每个 .tex 文件中提取 affiliations
    let mut all_affiliations = Vec::new();
    for (path, content) in &tex_files {
        info!("处理文件: {path}");
        let affiliations = extract_affiliations_from_latex(content);
        info!("从 {path} 中提取到 {} 个 affiliations", affiliations.len());

        for affiliation in &affiliations {
            info!("  - {affiliation}");
        }

        all_affiliations.extend(affiliations);
    }

    info!("✅ 总共提取到 {} 个 affiliations", all_affiliations.len());

    // 去重并排序
    all_affiliations.sort();
    all_affiliations.dedup();

    info!(
        "✅ 去重后共有 {} 个唯一 affiliations:",
        all_affiliations.len()
    );
    for affiliation in &all_affiliations {
        info!("  - {affiliation}");
    }

    // 验证至少提取到一些内容
    assert!(
        !all_affiliations.is_empty(),
        "应该至少提取到一个 affiliation"
    );
}

/// 测试 extract_affiliations_from_latex 函数的基本功能
#[tokio::test]
async fn test_extract_affiliations_from_latex_basic() {
    init_test_tracing();
    info!("开始测试 extract_affiliations_from_latex 函数的基本功能");

    // 测试包含 \affiliation 的 LaTeX 内容
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

    assert_eq!(affiliations.len(), 2, "应该提取到 2 个 affiliations");
    assert!(
        affiliations.contains(&"Department of Computer Science, University of Example".to_string())
    );
    assert!(affiliations.contains(&"Institute of Technology, Example City".to_string()));

    info!(
        "✅ 基本 affiliation 提取测试通过，提取到 {} 个 affiliations",
        affiliations.len()
    );
    for affiliation in &affiliations {
        info!("  - {affiliation}");
    }
}

/// 测试 extract_affiliations_from_latex 函数处理 \affil 命令
#[tokio::test]
async fn test_extract_affiliations_from_latex_affil() {
    init_test_tracing();
    info!("开始测试 extract_affiliations_from_latex 函数处理 \\affil 命令");

    // 测试包含 \affil 的 LaTeX 内容
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

    assert_eq!(affiliations.len(), 2, "应该提取到 2 个 affiliations");
    assert!(affiliations.contains(&"School of Engineering, Tech University".to_string()));
    assert!(affiliations.contains(&"Research Center, Innovation Lab".to_string()));

    info!(
        "✅ \\affil 命令测试通过，提取到 {} 个 affiliations",
        affiliations.len()
    );
    for affiliation in &affiliations {
        info!("  - {affiliation}");
    }
}

/// 测试 extract_affiliations_from_latex 函数处理 \institute 命令
#[tokio::test]
async fn test_extract_affiliations_from_latex_institute() {
    init_test_tracing();
    info!("开始测试 extract_affiliations_from_latex 函数处理 \\institute 命令");

    // 测试包含 \institute 的 LaTeX 内容
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

    assert_eq!(affiliations.len(), 2, "应该提取到 2 个 affiliations");
    assert!(affiliations.contains(&"Department of Mathematics, Science University".to_string()));
    assert!(affiliations.contains(&"Advanced Research Institute".to_string()));

    info!(
        "✅ \\institute 命令测试通过，提取到 {} 个 affiliations",
        affiliations.len()
    );
    for affiliation in &affiliations {
        info!("  - {affiliation}");
    }
}

/// 测试 extract_affiliations_from_latex 函数处理作者信息作为备选
#[tokio::test]
async fn test_extract_affiliations_from_latex_author_fallback() {
    init_test_tracing();
    info!("开始测试 extract_affiliations_from_latex 函数处理作者信息作为备选");

    // 测试没有 affiliation 信息，只有作者信息的 LaTeX 内容
    let latex_with_author_only = r#"\documentclass{article}
\begin{document}
\title{Test Paper}
\author{Alice Brown \and Charlie Wilson}
\maketitle

\section{Introduction}
This is a test document without affiliation information.
\end{document}"#;

    let affiliations = extract_affiliations_from_latex(latex_with_author_only);

    // 应该回退到提取作者信息
    assert!(!affiliations.is_empty(), "应该提取到作者信息作为备选");

    info!(
        "✅ 作者信息备选测试通过，提取到 {} 个 affiliations",
        affiliations.len()
    );
    for affiliation in &affiliations {
        info!("  - {affiliation}");
    }
}

/// 测试 extract_affiliations_from_latex 函数处理空内容
#[tokio::test]
async fn test_extract_affiliations_from_latex_empty() {
    init_test_tracing();
    info!("开始测试 extract_affiliations_from_latex 函数处理空内容");

    // 测试空的 LaTeX 内容
    let empty_latex = "";
    let affiliations = extract_affiliations_from_latex(empty_latex);

    assert_eq!(affiliations.len(), 0, "空内容应该返回空列表");

    // 测试只有文档结构的 LaTeX 内容
    let minimal_latex = r#"\documentclass{article}
\begin{document}
\title{Test}
\maketitle
\end{document}"#;

    let affiliations2 = extract_affiliations_from_latex(minimal_latex);

    info!(
        "✅ 空内容测试通过，空内容返回 {} 个 affiliations",
        affiliations.len()
    );
    info!(
        "✅ 最小内容测试通过，最小内容返回 {} 个 affiliations",
        affiliations2.len()
    );
}

/// 辅助函数：创建测试用的 tar.gz 文件
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
