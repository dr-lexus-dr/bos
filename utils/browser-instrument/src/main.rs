//! Insert cooperative scheduling points into the bundled WASI executable.
//! Appending a private global preserves every existing index. A checkpoint
//! before a function body and inside each loop preserves its operand stack.
use std::{env, fs, io};
use wasmparser::{Operator, Parser, Payload, TypeRef, Validator};
fn leb(mut n: u32, out: &mut Vec<u8>) {
    loop {
        let lo = (n & 127) as u8;
        n >>= 7;
        out.push(if n == 0 { lo } else { lo | 128 });
        if n == 0 {
            break;
        }
    }
}
fn counted(n: u32, data: &[u8], out: &mut Vec<u8>) {
    leb(n, out);
    out.extend_from_slice(data)
}
fn checkpoint(global: u32, yield_fn: u32) -> Vec<u8> {
    let mut o = Vec::new();
    o.push(0x23);
    leb(global, &mut o);
    o.extend([0x41, 1, 0x6b, 0x24]);
    leb(global, &mut o);
    o.push(0x23);
    leb(global, &mut o);
    o.extend([0x45, 0x04, 0x40, 0x41, 0x80, 0x04, 0x24]);
    leb(global, &mut o);
    o.push(0x10);
    leb(yield_fn, &mut o);
    o.extend([0x1a, 0x0b]);
    o
}
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<_> = env::args().collect();
    if args.len() != 3 {
        return Err(io::Error::other("expected input.wasm output.wasm").into());
    }
    let src = fs::read(&args[1])?;
    let mut out = src[..8].to_vec();
    let mut code = Vec::new();
    let mut globals = 0;
    let mut funcs = 0;
    let mut hook = None;
    let mut check = Vec::new();
    for item in Parser::new(0).parse_all(&src) {
        let payload = item?;
        match &payload {
            Payload::ImportSection(imports) => {
                for r in imports.clone() {
                    let i = r?;
                    match i.ty {
                        TypeRef::Func(_) => {
                            if (i.module, i.name) == ("wasi_snapshot_preview1", "sched_yield") {
                                hook = Some(funcs)
                            }
                            funcs += 1
                        }
                        TypeRef::Global(_) => globals += 1,
                        _ => {}
                    }
                }
            }
            Payload::GlobalSection(section) => {
                let r = section.range();
                let prefix = src[r.clone()].iter().position(|b| *b & 128 == 0).unwrap() + 1;
                let mut content = Vec::new();
                counted(
                    section.count() + 1,
                    &src[r.start + prefix..r.end],
                    &mut content,
                );
                content.extend([0x7f, 1, 0x41, 0x80, 4, 0x0b]);
                check = checkpoint(globals + section.count(), hook.ok_or("missing WASI yield")?);
                out.push(6);
                counted(content.len() as u32, &content, &mut out);
                continue;
            }
            Payload::CodeSectionStart { count, .. } => {
                leb(*count, &mut code);
                continue;
            }
            Payload::CodeSectionEntry(body) => {
                let mut ops = body.get_operators_reader()?;
                let r = body.range();
                let mut cursor = ops.original_position();
                let mut data = src[r.start..cursor].to_vec();
                data.extend(&check);
                while !ops.eof() {
                    if let Operator::Loop { .. } = ops.read()? {
                        let pos = ops.original_position();
                        data.extend_from_slice(&src[cursor..pos]);
                        data.extend(&check);
                        cursor = pos
                    }
                }
                data.extend_from_slice(&src[cursor..r.end]);
                counted(data.len() as u32, &data, &mut code);
                continue;
            }
            _ => {}
        }
        if !code.is_empty() {
            out.push(10);
            counted(code.len() as u32, &code, &mut out);
            code.clear();
        }
        if let Some((id, r)) = payload.as_section() {
            out.push(id);
            counted(r.len() as u32, &src[r], &mut out)
        }
    }
    Validator::new().validate_all(&out)?;
    fs::write(&args[2], out)?;
    Ok(())
}
