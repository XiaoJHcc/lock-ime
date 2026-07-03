//! 构建期图标生成：以 img/lock-ime-logo.png（16×16 像素画）为唯一来源，
//! 用最近邻整数倍放大（绝不插值）生成 exe 应用图标（多尺寸 .ico）与托盘图标
//! 的原始 RGBA。改了像素画只需重新导出 PNG 再编译即可。

use std::env;
use std::fs::File;
use std::io::BufWriter;
use std::path::Path;

const SRC: &str = "img/lock-ime-logo.png";

fn main() {
    println!("cargo:rerun-if-changed={SRC}");
    println!("cargo:rerun-if-changed=build.rs");

    let (w, h, rgba) = decode_rgba(SRC);
    let out_dir = env::var("OUT_DIR").expect("OUT_DIR");

    // 托盘图标：2× 最近邻放大，写成原始 RGBA + 尺寸常量供运行期直接使用。
    let (tw, th) = (w * 2, h * 2);
    let tray = nearest_scale(&rgba, w, h, tw, th);
    std::fs::write(Path::new(&out_dir).join("tray_rgba.bin"), &tray).unwrap();
    std::fs::write(
        Path::new(&out_dir).join("tray_meta.rs"),
        format!("const TRAY_W: u32 = {tw};\nconst TRAY_H: u32 = {th};\n"),
    )
    .unwrap();

    // 应用图标：多尺寸 .ico，全部为整数倍 => 像素边缘锐利。
    let ico_path = Path::new(&out_dir).join("lock-ime.ico");
    build_ico(&rgba, w, h, &[1, 2, 3, 4, 8, 16], &ico_path);

    if env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        let mut res = winresource::WindowsResource::new();
        res.set_icon(ico_path.to_str().unwrap());
        res.compile().expect("embed application icon");
    }
}

/// 解码 PNG 为 8-bit RGBA。源图请以 RGB/RGBA、8-bit 导出。
fn decode_rgba(path: &str) -> (u32, u32, Vec<u8>) {
    let decoder = png::Decoder::new(File::open(path).expect("open source png"));
    let mut reader = decoder.read_info().expect("read png info");
    let mut buf = vec![0u8; reader.output_buffer_size()];
    let info = reader.next_frame(&mut buf).expect("decode png frame");
    assert_eq!(info.bit_depth, png::BitDepth::Eight, "源图需为 8-bit");
    let bytes = &buf[..info.buffer_size()];
    let rgba = match info.color_type {
        png::ColorType::Rgba => bytes.to_vec(),
        png::ColorType::Rgb => {
            let mut v = Vec::with_capacity((info.width * info.height * 4) as usize);
            for px in bytes.chunks_exact(3) {
                v.extend_from_slice(px);
                v.push(255);
            }
            v
        }
        other => panic!("不支持的 PNG 颜色类型 {other:?}，请导出为 RGBA"),
    };
    (info.width, info.height, rgba)
}

/// 最近邻缩放（整数倍时即像素完美放大，无任何插值）。
fn nearest_scale(src: &[u8], sw: u32, sh: u32, dw: u32, dh: u32) -> Vec<u8> {
    let mut dst = vec![0u8; (dw * dh * 4) as usize];
    for y in 0..dh {
        let sy = y * sh / dh;
        for x in 0..dw {
            let sx = x * sw / dw;
            let si = ((sy * sw + sx) * 4) as usize;
            let di = ((y * dw + x) * 4) as usize;
            dst[di..di + 4].copy_from_slice(&src[si..si + 4]);
        }
    }
    dst
}

/// 用若干整数倍尺寸构建多尺寸 .ico。
fn build_ico(src: &[u8], sw: u32, sh: u32, scales: &[u32], out: &Path) {
    let mut dir = ico::IconDir::new(ico::ResourceType::Icon);
    for &s in scales {
        let (dw, dh) = (sw * s, sh * s);
        let data = nearest_scale(src, sw, sh, dw, dh);
        let img = ico::IconImage::from_rgba_data(dw, dh, data);
        dir.add_entry(ico::IconDirEntry::encode(&img).expect("encode ico entry"));
    }
    dir.write(BufWriter::new(File::create(out).expect("create ico")))
        .expect("write ico");
}
