use std::{
    path::{Path, PathBuf},
    process::{Command, Stdio},
    collections::HashMap,
    fs,
    io::{self, Write},
    env,
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, from_str};

#[derive(Debug, Serialize, Deserialize)]
struct VersionJson {
    id: String,
    #[serde(rename = "mainClass")]
    main_class: String,
    #[serde(rename = "minecraftArguments")]
    minecraft_arguments: Option<String>,
    arguments: Option<Arguments>,
    libraries: Vec<Library>,
    assets: Option<String>,
    #[serde(rename = "type")]
    version_type: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
struct Arguments {
    game: Vec<GameArgument>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(untagged)]
enum GameArgument {
    String(String),
    Object(HashMap<String, Value>),
}

#[derive(Debug, Serialize, Deserialize)]
struct Library {
    name: String,
    rules: Option<Vec<Rule>>,
    natives: Option<HashMap<String, String>>,
}

#[derive(Debug, Serialize, Deserialize)]
struct Rule {
    action: String,
    os: Option<Os>,
}

#[derive(Debug, Serialize, Deserialize)]
struct Os {
    name: Option<String>,
    arch: Option<String>,
}

struct LaunchOptions {
    java_path: String,
    memory: Option<u32>,
    use_system_memory: bool,
}

fn main() {
    let mc_path = ".minecraft";
    let version_name = "1.20.1";
    let player_name = "Player123";

    let options = LaunchOptions {
        java_path: "java".to_string(),
        memory: Some(4096),
        use_system_memory: false,
    };

    if let Err(e) = launch_minecraft(mc_path, version_name, player_name, &options) {
        eprintln!("Failed to launch Minecraft: {}", e);
    }
}

fn launch_minecraft(
    mc_path: &str,
    version_name: &str,
    player_name: &str,
    options: &LaunchOptions,
) -> io::Result<()> {
    // Normalize path
    let mc_path = normalize_path(mc_path)?;

    // Read version JSON file
    let version_json_path = mc_path
        .join("versions")
        .join(version_name)
        .join(format!("{}.json", version_name));
    let version_json = read_version_json(&version_json_path)?;

    // Build libraries path
    let libraries = build_libraries_path(&mc_path, &version_json)?;

    // Build game arguments
    let game_args = build_game_arguments(&mc_path, version_name, player_name, &version_json);

    // Build Java command
    let java_command = build_java_command(
        &mc_path,
        version_name,
        &version_json.main_class,
        &libraries,
        &game_args,
        options,
    );

    println!("Launching Minecraft with command: {}", java_command);

    // Execute command
    let mut child = Command::new("cmd")
        .arg("/K")
        .arg(&java_command)
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .spawn()?;

    println!("Minecraft launched with PID: {}", child.id());

    Ok(())
}

fn normalize_path(mc_path: &str) -> io::Result<PathBuf> {
    let path = mc_path.replace('/', "\\");
    /*if path == ".minecraft" {
        let current_dir = ".minecarft"; //env::current_dir()?;  // 获取当前工作目录
        Ok(current_dir.join(".minecraft"))
    } else {*/
        Ok(PathBuf::from(path))
    //}
}

fn read_version_json(path: &Path) -> io::Result<VersionJson> {
    let content = fs::read_to_string(path)
        .map_err(|e| io::Error::new(io::ErrorKind::NotFound, 
            format!("无法读取文件 {}: {}", path.display(), e)))?;
    
    from_str(&content).map_err(|e| io::Error::new(
        io::ErrorKind::InvalidData, 
        format!("无效的JSON格式 {}: {}", path.display(), e)))
}

fn build_libraries_path(mc_path: &Path, version_json: &VersionJson) -> io::Result<String> {
    let mut libraries = vec![mc_path
        .join("versions")
        .join(&version_json.id)
        .join(format!("{}.jar", version_json.id))];

    for lib in &version_json.libraries {
        if !check_library_rules(lib) {
            continue;
        }

        if let Some(lib_path) = get_library_path(mc_path, lib) {
            libraries.push(lib_path);
        }
    }

    Ok(libraries
        .into_iter()
        .map(|p| p.to_string_lossy().into_owned())
        .collect::<Vec<_>>()
        .join(";"))
}

fn check_library_rules(lib: &Library) -> bool {
    if lib.rules.is_none() || lib.rules.as_ref().unwrap().is_empty() {
        return true;
    }

    let os_name = "windows";
    let os_arch = if cfg!(target_arch = "x86_64") {
        "x86_64"
    } else {
        "x86"
    };

    let mut should_include = true;

    for rule in lib.rules.as_ref().unwrap() {
        if rule.action == "allow" {
            if rule.os.is_none() {
                should_include = true;
                continue;
            }

            let os = rule.os.as_ref().unwrap();
            if os.name.as_deref() == Some(os_name) {
                if let Some(arch) = &os.arch {
                    should_include = arch == os_arch;
                } else {
                    should_include = true;
                }
            } else {
                should_include = false;
            }
        } else if rule.action == "disallow" {
            if rule.os.is_none() {
                should_include = false;
                continue;
            }

            if rule.os.as_ref().unwrap().name.as_deref() == Some(os_name) {
                should_include = false;
            }
        }
    }

    should_include
}

fn get_library_path(mc_path: &Path, lib: &Library) -> Option<PathBuf> {
    let parts: Vec<&str> = lib.name.split(':').collect();
    if parts.len() < 3 {
        return None;
    }

    let group_path = parts[0].replace('.', &std::path::MAIN_SEPARATOR.to_string());
    let artifact_id = parts[1];
    let version = parts[2];

    let base_path = mc_path
        .join("libraries")
        .join(group_path)
        .join(artifact_id)
        .join(version);
    let base_file = format!("{}-{}", artifact_id, version);

    // Check for natives
    if let Some(natives) = &lib.natives {
        if let Some(windows_native) = natives.get("windows") {
            let classifier = windows_native.replace("${arch}", if cfg!(target_arch = "x86_64") { "64" } else { "32" });
            let native_path = base_path.join(format!("{}-{}.jar", base_file, classifier));

            if native_path.exists() {
                return Some(native_path);
            }
        }
    }

    // Default to regular jar
    let jar_path = base_path.join(format!("{}.jar", base_file));
    if jar_path.exists() {
        return Some(jar_path);
    }

    None
}

fn build_game_arguments(
    mc_path: &Path,
    version_name: &str,
    player_name: &str,
    version_json: &VersionJson,
) -> String {
    let assets_path = mc_path.join("assets");
    let assets_index = version_json.assets.as_deref().unwrap_or("");

    let mut args = String::new();

    // Handle older versions with minecraftArguments
    if let Some(minecraft_args) = &version_json.minecraft_arguments {
        args.push_str(minecraft_args);
    }

    // Handle newer versions with arguments.game
    if let Some(arguments) = &version_json.arguments {
        for arg in &arguments.game {
            if let GameArgument::String(s) = arg {
                args.push(' ');
                args.push_str(s);
            }
        }
    }

    // Replace placeholders
    let replacements = [
        ("${auth_player_name}", player_name),
        ("${version_name}", version_name),
        ("${game_directory}", mc_path.to_str().unwrap_or("")),
        ("${assets_root}", assets_path.to_str().unwrap_or("")),
        ("${assets_index_name}", assets_index),
        ("${auth_uuid}", "00000000-0000-0000-0000-000000000000"),
        ("${auth_access_token}", "00000000000000000000000000000000"),
        ("${user_type}", "legacy"),
        ("${version_type}", "WMML 0.1.26"),
    ];

    for (placeholder, value) in replacements {
        args = args.replace(placeholder, value);
    }

    args.trim().to_string()
}

fn build_java_command(
    mc_path: &Path,
    version_name: &str,
    main_class: &str,
    libraries: &str,
    game_args: &str,
    options: &LaunchOptions,
) -> String {
    // Memory settings
    let memory_settings = if !options.use_system_memory && options.memory.is_some() {
        format!("-Xmx{}M -Xms{}M ", options.memory.unwrap(), options.memory.unwrap())
    } else {
        String::new()
    };

    // Common JVM arguments
    let common_args = [
        "-Dfile.encoding=GB18030",
        "-Dsun.stdout.encoding=GB18030",
        "-Dsun.stderr.encoding=GB18030",
        "-Djava.rmi.server.useCodebaseOnly=true",
        "-Dcom.sun.jndi.rmi.object.trustURLCodebase=false",
        "-Dcom.sun.jndi.cosnaming.object.trustURLCodebase=false",
        "-Dlog4j2.formatMsgNoLookups=true",
        &format!(
            "-Dlog4j.configurationFile={}",
            mc_path
                .join("versions")
                .join(version_name)
                .join("log4j2.xml")
                .to_str()
                .unwrap_or("")
        ),
        &format!(
            "-Dminecraft.client.jar={}",
            mc_path
                .join("versions")
                .join(version_name)
                .join(format!("{}.jar", version_name))
                .to_str()
                .unwrap_or("")
        ),
        "-XX:+UnlockExperimentalVMOptions",
        "-XX:+UseG1GC",
        "-XX:G1NewSizePercent=20",
        "-XX:G1ReservePercent=20",
        "-XX:MaxGCPauseMillis=50",
        "-XX:G1HeapRegionSize=32m",
        "-XX:-UseAdaptiveSizePolicy",
        "-XX:-OmitStackTraceInFastThrow",
        "-XX:-DontCompileHugeMethods",
        "-Dfml.ignoreInvalidMinecraftCertificates=true",
        "-Dfml.ignorePatchDiscrepancies=true",
        "-XX:HeapDumpPath=MojangTricksIntelDriversForPerformance_javaw.exe_minecraft.exe.heapdump",
        &format!(
            "-Djava.library.path={}",
            mc_path
                .join("versions")
                .join(version_name)
                .join("natives-windows-x86_64")
                .to_str()
                .unwrap_or("")
        ),
        &format!(
            "-Djna.tmpdir={}",
            mc_path
                .join("versions")
                .join(version_name)
                .join("natives-windows-x86_64")
                .to_str()
                .unwrap_or("")
        ),
        &format!(
            "-Dorg.lwjgl.system.SharedLibraryExtractPath={}",
            mc_path
                .join("versions")
                .join(version_name)
                .join("natives-windows-x86_64")
                .to_str()
                .unwrap_or("")
        ),
        &format!(
            "-Dio.netty.native.workdir={}",
            mc_path
                .join("versions")
                .join(version_name)
                .join("natives-windows-x86_64")
                .to_str()
                .unwrap_or("")
        ),
        "-Dminecraft.launcher.brand=WMML",
        "-Dminecraft.launcher.version=0.1.26",
    ]
    .join(" ");

    // Construct full command
    format!(
        "{} {} {} -cp {} {} {}",
        options.java_path, memory_settings, common_args, libraries, main_class, game_args
    )
}