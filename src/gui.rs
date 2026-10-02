//! Minimal loopback browser frontend. It calls the library render API directly.

use anyhow::{bail, Context, Result};
use serde::Deserialize;
use std::{
    io::{Read, Write},
    net::{TcpListener, TcpStream},
    process::Command,
    thread,
};

const PAGE: &str = r#"<!doctype html><html><head><meta charset="utf-8"><title>Sono-Renderer</title>
<style>body{font:14px system-ui;background:#171b24;color:#e8edf6;margin:20px}main{max-width:1100px;margin:auto}fieldset{border:1px solid #465064;margin:10px 0;padding:12px}label{display:inline-block;margin:5px 12px 5px 0}input{background:#242b38;color:#fff;border:1px solid #596579;padding:5px}input[type=text]{width:280px}canvas{max-width:100%;background:#000}button{padding:8px 14px;margin:5px}#status{white-space:pre-wrap}</style></head><body><main>
<h1>Sono-Renderer preview</h1>
<fieldset><legend>Inputs</legend><label>Engine <input id="engine" type="text" placeholder="engine.zip"></label><label>Resources <input id="resources" type="text" placeholder="resources.scp"></label><br><label>Skin override <input id="skin" type="text" placeholder="engine default"></label><label>Level <input id="level" type="text" placeholder="level.zip"></label><label>BGM <input id="music" type="text" placeholder="music.mp3"></label><label>Output <input id="output" type="text" placeholder="render.mp4"></label></fieldset>
<fieldset><legend>Timeline and output</legend><label>Width <input id="width" type="number" value="640"></label><label>Height <input id="height" type="number" value="360"></label><label>FPS <input id="fps" type="number" value="12"></label><label>Start <input id="start" type="number" value="14" step="0.01"></label><label>Duration <input id="duration" type="number" value="2" step="0.01"></label><label>Preview timestamp <input id="preview" type="number" value="14" step="0.01"></label><br><label>Level options (INDEX=VALUE, one per line)<br><textarea id="options" rows="3" cols="32"></textarea></label></fieldset>
<fieldset><legend>Render layers</legend><label><input id="particles" type="checkbox" checked> Particles</label><label><input id="sfx" type="checkbox" checked> SFX</label><label><input id="bgm" type="checkbox" checked> BGM</label><label><input id="ui" type="checkbox"> Renderer UI</label><br><label><input id="primary_on" type="checkbox" checked> Primary metric</label><label><input id="secondary_on" type="checkbox"> Secondary metric</label><label><input id="combo_on" type="checkbox"> Combo</label><label><input id="judgment_on" type="checkbox"> Judgment</label><label><input id="progress_on" type="checkbox" checked> Progress</label></fieldset>
<fieldset><legend>Metric providers</legend><label>Primary label <input id="primary_label" type="text" value="POINTS"></label><label>Primary provider <input id="primary_provider" type="text" value="fixed:0"></label><label>Maximum <input id="primary_max" type="number" placeholder="optional"></label><br><label>Secondary label <input id="secondary_label" type="text" value="SECONDARY"></label><label>Secondary provider <input id="secondary_provider" type="text" value="fixed:0"></label><label>Maximum <input id="secondary_max" type="number" placeholder="optional"></label><br><label>Combo provider <input id="combo_provider" type="text" value="fixed:0"></label><label>Judgment label <input id="judgment_label" type="text" value="JUDGMENT"></label><label>Judgment provider <input id="judgment_provider" type="text" value="fixed:0"></label></fieldset>
<button onclick="previewFrame()">Render preview frame</button><button onclick="renderVideo()">Render video</button><div id="status"></div><canvas id="canvas"></canvas>
<script>
function provider(s){let p=s.trim();if(p==='progress'||p==='accuracy')return{kind:p};if(p.startsWith('memory:')){let a=p.slice(7).split(':');return{kind:'memory',block_id:Number(a[0]),index:Number(a[1])}}if(p.startsWith('external:'))return{kind:'external',value:Number(p.slice(9))};if(p.startsWith('engine:'))return{kind:'engine-metric',key:p.slice(7)};if(p.startsWith('judgment:'))return{kind:'judgment-derived',mapping:p.slice(9)};return{kind:'fixed',value:Number(p.replace(/^fixed:/,''))}}
function val(id){return document.getElementById(id).value} function checked(id){return document.getElementById(id).checked}
function config(){let opts=val('options').split(/\r?\n/).filter(x=>x.trim()).map(x=>{let a=x.split('=');return{index:Number(a[0]),value:Number(a[1])}});return{engine:val('engine'),resources:val('resources'),level:val('level'),skin:val('skin')||null,music:val('music')||null,output:val('output')||null,start_time:Number(val('start')),duration:Number(val('duration')),fps:Number(val('fps')),width:Number(val('width')),height:Number(val('height')),level_options:opts,layers:{particles:checked('particles'),sfx:checked('sfx'),bgm:checked('bgm')},ui:{enabled:checked('ui'),primary_metric:{enabled:checked('primary_on'),label:val('primary_label'),provider:provider(val('primary_provider')),maximum:val('primary_max')?Number(val('primary_max')):null},secondary_metric:{enabled:checked('secondary_on'),label:val('secondary_label'),provider:provider(val('secondary_provider')),maximum:val('secondary_max')?Number(val('secondary_max')):null},combo:{enabled:checked('combo_on'),label:'COMBO',provider:provider(val('combo_provider'))},judgment:{enabled:checked('judgment_on'),label:val('judgment_label'),provider:provider(val('judgment_provider'))},progress:checked('progress_on')},trace_entity_id:null}}
async function previewFrame(){try{let c=config();c.ui.enabled=checked('ui');let r=await fetch('/api/preview',{method:'POST',headers:{'content-type':'application/json'},body:JSON.stringify({config:c,time:Number(val('preview'))})});if(!r.ok)throw Error(await r.text());let w=Number(r.headers.get('x-frame-width')),h=Number(r.headers.get('x-frame-height')),b=new Uint8ClampedArray(await r.arrayBuffer()),ctx=document.getElementById('canvas').getContext('2d');let im=new ImageData(b,w,h);document.getElementById('canvas').width=w;document.getElementById('canvas').height=h;ctx.putImageData(im,0,0);document.getElementById('status').textContent='Preview rendered at '+(Math.ceil(Number(val('preview'))*Number(val('fps')))/Number(val('fps'))).toFixed(4)+'s'}catch(e){document.getElementById('status').textContent='Error: '+e}}
async function renderVideo(){try{let r=await fetch('/api/render',{method:'POST',headers:{'content-type':'application/json'},body:JSON.stringify(config())});let t=await r.text();if(!r.ok)throw Error(t);document.getElementById('status').textContent='Video export complete\n'+t}catch(e){document.getElementById('status').textContent='Error: '+e}}
</script></main></body></html>"#;

#[derive(Deserialize)]
struct PreviewRequest {
    config: crate::render::RenderConfig,
    time: f64,
}

/// Browser text inputs commonly receive paths copied with shell quotes. A
/// command-line shell removes those delimiters before Rust sees argv; JSON
/// does not, so apply the same token cleanup at the GUI boundary only.
fn normalize_gui_config_paths(config: &mut crate::render::RenderConfig) {
    for path in [
        Some(&mut config.engine),
        Some(&mut config.resources),
        Some(&mut config.level),
        config.music.as_mut(),
        config.output.as_mut(),
    ]
    .into_iter()
    .flatten()
    {
        let raw = path.to_string_lossy();
        let trimmed = raw.trim();
        let unquoted = if trimmed.len() >= 2
            && ((trimmed.starts_with('"') && trimmed.ends_with('"'))
                || (trimmed.starts_with('\'') && trimmed.ends_with('\'')))
        {
            &trimmed[1..trimmed.len() - 1]
        } else {
            trimmed
        };
        *path = unquoted.into();
    }
}

pub fn launch() -> Result<()> {
    let listener = TcpListener::bind(("127.0.0.1", 0)).context("binding local preview UI")?;
    let address = listener.local_addr()?;
    let url = format!("http://{address}/");
    open_browser(&url)?;
    eprintln!("Sono-Renderer GUI: {url}");
    for stream in listener.incoming() {
        match stream {
            Ok(stream) => {
                thread::spawn(|| {
                    let _ = handle(stream);
                });
            }
            Err(error) => eprintln!("GUI connection failed: {error}"),
        }
    }
    Ok(())
}

#[cfg(windows)]
fn open_browser(url: &str) -> Result<()> {
    Command::new("cmd")
        .args(["/C", "start", "", url])
        .spawn()
        .context("opening browser")?;
    Ok(())
}

#[cfg(target_os = "macos")]
fn open_browser(url: &str) -> Result<()> {
    Command::new("open")
        .arg(url)
        .spawn()
        .context("opening browser")?;
    Ok(())
}

#[cfg(all(unix, not(target_os = "macos")))]
fn open_browser(url: &str) -> Result<()> {
    Command::new("xdg-open")
        .arg(url)
        .spawn()
        .context("opening browser")?;
    Ok(())
}

fn handle(mut stream: TcpStream) -> Result<()> {
    let mut request = Vec::new();
    let mut buffer = [0_u8; 4096];
    let header_end;
    loop {
        let count = stream.read(&mut buffer)?;
        if count == 0 {
            bail!("client closed incomplete HTTP request");
        }
        request.extend_from_slice(&buffer[..count]);
        if let Some(position) = request.windows(4).position(|window| window == b"\r\n\r\n") {
            header_end = position + 4;
            break;
        }
        if request.len() > 64 * 1024 {
            bail!("HTTP headers exceed size limit");
        }
    }
    let headers = std::str::from_utf8(&request[..header_end])?;
    let mut lines = headers.lines();
    let request_line = lines.next().context("HTTP request line is missing")?;
    let mut parts = request_line.split_whitespace();
    let method = parts.next().unwrap_or("").to_owned();
    let path = parts.next().unwrap_or("/").to_owned();
    let content_length = lines
        .filter_map(|line| line.split_once(':'))
        .find(|(key, _)| key.eq_ignore_ascii_case("content-length"))
        .map(|(_, value)| value.trim().parse::<usize>())
        .transpose()?
        .unwrap_or(0);
    if content_length > 4 * 1024 * 1024 {
        bail!("HTTP body exceeds size limit");
    }
    while request.len() < header_end + content_length {
        let count = stream.read(&mut buffer)?;
        if count == 0 {
            bail!("client closed incomplete HTTP body");
        }
        request.extend_from_slice(&buffer[..count]);
    }
    let body = &request[header_end..header_end + content_length];
    match (method.as_str(), path.as_str()) {
        ("GET", "/") => respond(
            &mut stream,
            200,
            "text/html; charset=utf-8",
            PAGE.as_bytes(),
            &[],
        ),
        ("POST", "/api/preview") => {
            let mut request: PreviewRequest = match serde_json::from_slice(body) {
                Ok(request) => request,
                Err(error) => {
                    return respond(
                        &mut stream,
                        400,
                        "text/plain; charset=utf-8",
                        error.to_string().as_bytes(),
                        &[],
                    )
                }
            };
            normalize_gui_config_paths(&mut request.config);
            let frame = match request.config.render_frame(request.time) {
                Ok(frame) => frame,
                Err(error) => {
                    return respond(
                        &mut stream,
                        422,
                        "text/plain; charset=utf-8",
                        format!("{error:#}").as_bytes(),
                        &[],
                    )
                }
            };
            respond(
                &mut stream,
                200,
                "application/octet-stream",
                &frame.rgb,
                &[
                    ("X-Frame-Width", request.config.width.to_string()),
                    ("X-Frame-Height", request.config.height.to_string()),
                ],
            )
        }
        ("POST", "/api/render") => {
            let mut config: crate::render::RenderConfig = match serde_json::from_slice(body) {
                Ok(config) => config,
                Err(error) => {
                    return respond(
                        &mut stream,
                        400,
                        "text/plain; charset=utf-8",
                        error.to_string().as_bytes(),
                        &[],
                    )
                }
            };
            normalize_gui_config_paths(&mut config);
            let report = match config.render_video() {
                Ok(report) => report,
                Err(error) => {
                    return respond(
                        &mut stream,
                        422,
                        "text/plain; charset=utf-8",
                        format!("{error:#}").as_bytes(),
                        &[],
                    )
                }
            };
            let json = serde_json::to_vec_pretty(&report)?;
            respond(&mut stream, 200, "application/json", &json, &[])
        }
        _ => respond(&mut stream, 404, "text/plain", b"not found", &[]),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{io::Read, net::TcpListener, path::PathBuf};

    #[test]
    fn browser_config_path_with_spaces_and_shell_quotes_reaches_engine_zip_loader() {
        let repo = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
        let engine = repo.join("TestingSuite/Next RUSH/engine/Next RUSH.zip");
        let body = serde_json::to_vec(&serde_json::json!({
            "engine": format!("  \"{}\"  ", engine.display()),
            "resources": "resources.scp",
            "level": "level.zip",
        }))
        .unwrap();

        let mut config: crate::render::RenderConfig = serde_json::from_slice(&body).unwrap();
        assert_eq!(
            config.engine,
            PathBuf::from(format!("  \"{}\"  ", engine.display()))
        );
        assert_ne!(config.engine, PathBuf::from(&engine));
        normalize_gui_config_paths(&mut config);

        assert_eq!(config.engine, PathBuf::from(&engine));
        assert!(config.engine.is_file());
        let package = config.load_engine_package().unwrap();
        assert!(!package.watch.nodes.is_empty());
    }

    #[test]
    fn http_render_decodes_spaced_engine_path_to_the_same_cli_file_path() {
        let repo = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
        let engine = repo.join("TestingSuite/Next RUSH/engine/Next RUSH.zip");
        let missing_resources = repo.join("TestingSuite/absent resources.scp");
        let body = serde_json::to_vec(&serde_json::json!({
            "engine": format!("\"{}\"", engine.display()),
            "resources": missing_resources,
            "level": "unused level.zip",
            "output": "unused output.mp4",
            "layers": {"particles":false,"sfx":false,"bgm":false}
        }))
        .unwrap();

        let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let address = listener.local_addr().unwrap();
        let server = std::thread::spawn(move || {
            let (stream, _) = listener.accept().unwrap();
            handle(stream).unwrap();
        });
        let mut client = TcpStream::connect(address).unwrap();
        write!(
            client,
            "POST /api/render HTTP/1.1\r\nHost: localhost\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
            body.len()
        )
        .unwrap();
        client.write_all(&body).unwrap();
        let mut response = String::new();
        client.read_to_string(&mut response).unwrap();
        server.join().unwrap();

        assert!(response.starts_with("HTTP/1.1 422"), "{response}");
        assert!(
            response.contains("resources input is not an accessible file"),
            "{response}"
        );
        assert!(
            !response.contains("engine input is not an accessible file"),
            "{response}"
        );
    }
}

fn respond(
    stream: &mut TcpStream,
    status: u16,
    content_type: &str,
    body: &[u8],
    headers: &[(&str, String)],
) -> Result<()> {
    let reason = match status {
        200 => "OK",
        400 => "Bad Request",
        404 => "Not Found",
        422 => "Unprocessable Entity",
        _ => "Internal Server Error",
    };
    write!(stream, "HTTP/1.1 {status} {reason}\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close\r\n", body.len())?;
    for (name, value) in headers {
        write!(stream, "{name}: {value}\r\n")?;
    }
    stream.write_all(b"\r\n")?;
    stream.write_all(body)?;
    Ok(())
}
