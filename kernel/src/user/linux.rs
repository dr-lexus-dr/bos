//! Linux x86-64 syscall personality for static musl PIE userspace on the BSP.
//! No Linux kernel is bundled: calls use bOS page tables, scheduler and VFS.
use super::*;
use alloc::{string::{String, ToString}, vec};
use crate::fs::{self, VFS};
use x86_64::registers::model_specific::Msr;
const LIMIT: u64 = STACK_BOTTOM;
const MMAP_START: u64 = USER_BASE + 160 * 1024 * 1024;

#[derive(Clone)]
pub(super) struct State {
    pub enabled: bool,
    pub fs: u64,
    pub cwd: String,
    brk: u64,
    heap_start: u64,
    mmap: u64,
    paths: [Option<String>; 64],
    offsets: [usize; 64],
    cloexec: u64,
    pgid: u64,
    actions: [[u8; 32]; 65],
    mask: u64,
    umask: u32,
    executable: String,
    termios: [u8; 36],
    poll_deadline: Option<u64>,
}
impl State {
    pub fn new() -> Self {
        Self { enabled: false, fs: 0, cwd: String::from("/users/default"), brk: USER_BASE + 64*1024*1024,
            heap_start: USER_BASE + 64*1024*1024, mmap: MMAP_START,
            paths: core::array::from_fn(|_| None), offsets: [0;64], cloexec: 0, pgid: 1,
            actions: [[0;32];65], mask: 0, umask: 0o022, executable: String::new(), termios: [0;36], poll_deadline: None }
    }
}
pub(super) unsafe fn set_fs(value: u64) { Msr::new(0xc0000100).write(value); }
pub(super) unsafe fn enable_syscall() {
    unsafe extern "C" { fn linux_syscall(); }
    Msr::new(0xc0000081).write((gdt::KERNEL_CS.0 as u64) << 32);
    Msr::new(0xc0000082).write(linux_syscall as *const () as u64);
    Msr::new(0xc0000084).write(0x700); // TF, IF, DF
    Efer::update(|f| f.insert(EferFlags::SYSTEM_CALL_EXTENSIONS));
}
pub(super) fn environment() -> Vec<String> {
    ["PATH=/bin:/system", "BOS_ROOT=/", "HOME=/users/default", "USER=default", "LOGNAME=default",
     "SHELL=/bin/nash", "TERM=xterm-256color", "LANG=C", "LC_ALL=C", "PS1=nash> "]
        .iter().map(|s| String::from(*s)).collect()
}
pub(super) fn initialize(p: &mut Program, elf: &Executable<'_>, argv: &[String], env: &[String]) -> Result<(), &'static str> {
    p.linux.enabled = true;
    p.linux.executable = argv.first().cloned().unwrap_or_default();
    p.linux.brk = p.pages.iter().map(|(v,_,_)| v+PAGE).filter(|v| *v <= IMAGE_END).max().unwrap_or(USER_BASE);
    p.linux.heap_start = p.linux.brk;
    let mut sp = STACK_TOP;
    let mut strings = |items: &[String]| -> Result<Vec<u64>, &'static str> {
        let mut pointers = Vec::new();
        for s in items {
            sp = sp.checked_sub(s.len() as u64 + 1).ok_or("argument overflow")?;
            p.write_user(sp, s.as_bytes()).map_err(|_| "argument stack overflow")?;
            p.write_user(sp+s.len() as u64, &[0]).map_err(|_| "argument stack overflow")?;
            pointers.push(sp);
        }
        Ok(pointers)
    };
    let args = strings(argv)?;
    let vars = strings(env)?;
    sp = (sp - 16) & !15;
    let random = sp;
    let mut entropy = [0u8;16];
    random_bytes(&mut entropy).map_err(|_| "hardware RNG unavailable")?;
    p.write_user(random,&entropy).map_err(|_| "stack overflow")?;
    let mut words = vec![args.len() as u64];
    words.extend(args); words.push(0); words.extend(vars); words.push(0);
    for (key,value) in [(3,elf.phdr),(4,56),(5,elf.phnum as u64),(6,PAGE),(7,0),(9,elf.entry),
        (11,0),(12,0),(13,0),(14,0),(17,100),(23,0),(25,random),(0,0)] {
        words.extend([key,value]);
    }
    sp = (sp - (words.len()*8) as u64) & !15;
    for (i,w) in words.iter().enumerate() { p.write_user(sp+i as u64*8,&w.to_le_bytes()).map_err(|_| "stack overflow")?; }
    p.context.rsp = sp;
    Ok(())
}
pub(super) fn random_bytes(out: &mut [u8]) -> Result<(), i64> {
    if unsafe { core::arch::x86_64::__cpuid(1) }.ecx & (1<<30) == 0 { return Err(-38); }
    for chunk in out.chunks_mut(8) {
        let mut value = 0u64;
        let mut good = false;
        for _ in 0..32 { if unsafe { core::arch::x86_64::_rdrand64_step(&mut value) } != 0 { good = true; break; } }
        if !good { return Err(-5); }
        chunk.copy_from_slice(&value.to_le_bytes()[..chunk.len()]);
    }
    Ok(())
}
fn error(e: fs::FileSystemError) -> i64 { crate::syscall::fs_err(e) as i64 }
fn read64(p: &Program, a: u64) -> Result<u64,i64> { let mut b=[0;8]; p.read_user(a,&mut b).map_err(|_| -14i64)?; Ok(u64::from_le_bytes(b)) }
fn read32(p: &Program, a: u64) -> Result<u32,i64> { let mut b=[0;4]; p.read_user(a,&mut b).map_err(|_| -14i64)?; Ok(u32::from_le_bytes(b)) }
fn path_at(p: &Program, dir: u64, address: u64) -> Result<String,i64> {
    let name=p.path(address)?;
    let cwd=if name.starts_with('/') || dir as i64 == -100 { &p.linux.cwd }
        else { p.linux.paths.get(dir as usize).and_then(|p|p.as_ref()).ok_or(-9i64)? };
    fs::normalize_path(cwd,&name).map_err(error)
}
fn fd_slot(p: &Program, minimum: usize) -> Result<usize,i64> {
    (minimum..64).find(|i|p.descriptors[*i].is_none()).ok_or(-24)
}
fn open(p: &mut Program, dir: u64, address: u64, flags: u64, mode: u64) -> Result<i64,i64> {
    let path=path_at(p,dir,address)?;
    let slot=fd_slot(p,0)?;
    let flags=flags as u32;
    let handle=match path.as_str() {
        "/dev/null" => Handle::Null,
        "/dev/urandom" | "/dev/random" => Handle::Random,
        "/dev/tty" => Handle::ConsoleRead,
        _ => {
            let mut vfs=VFS.lock();
            if vfs.stat(&path).is_ok_and(|s| s.is_dir()) {
                if flags & fs::O_ACCMODE != 0 { return Err(-21); }
                Handle::Directory
            } else {
                let fd=vfs.open(&path,flags & (3|0x40|0x80|0x200|0x400|0x10000)).map_err(error)?;
                if flags & fs::O_CREAT != 0 { let _=vfs.chmod(&path,mode as u32 & 0o777 & !p.linux.umask); }
                Handle::File(fd)
            }
        }
    };
    p.descriptors[slot]=Some(Arc::new(handle));
    p.linux.paths[slot]=Some(path);
    p.linux.offsets[slot]=0;
    p.linux.cloexec &= !(1<<slot);
    if flags & 0x80000 != 0 { p.linux.cloexec |= 1<<slot; }
    Ok(slot as i64)
}
fn stat_bytes(s: &fs::FileStat) -> [u8;144] {
    let mut b=[0;144];
    for (o,v) in [(0,1),(8,s.inode),(16,s.links as u64),(40,0),(48,s.size),(56,PAGE),(64,(s.size+511)/512),
        (72,s.atime),(88,s.mtime),(104,s.ctime)] { b[o..o+8].copy_from_slice(&v.to_le_bytes()); }
    for (o,v) in [(24,s.mode),(28,s.uid),(32,s.gid)] { b[o..o+4].copy_from_slice(&v.to_le_bytes()); }
    b
}
fn stat_fd(p:&Program,fd:u64)->Result<fs::FileStat,i64> {
    let handle=p.descriptor(fd)?;
    match &*handle {
        Handle::File(f)=>VFS.lock().fstat(*f).map_err(error),
        Handle::Directory=>VFS.lock().stat(p.linux.paths[fd as usize].as_ref().ok_or(-9i64)?).map_err(error),
        _=>Ok(fs::FileStat{inode:fd+1,size:0,mode:if matches!(&*handle,Handle::PipeRead(_)|Handle::PipeWrite(_)){fs::S_IFIFO|0o600}else{fs::S_IFCHR|0o666},uid:0,gid:0,atime:0,mtime:0,ctime:0,links:1})
    }
}
fn duplicate(p:&mut Program,from:u64,to:usize,cloexec:bool)->Result<i64,i64> {
    if to>=64 {return Err(-9);}
    p.descriptors[to]=Some(p.descriptor(from)?);
    p.linux.paths[to]=p.linux.paths[from as usize].clone();
    p.linux.offsets[to]=p.linux.offsets[from as usize];
    p.linux.cloexec &= !(1<<to); if cloexec {p.linux.cloexec |= 1<<to;}
    Ok(to as i64)
}
fn map(p:&mut Program,start:u64,length:u64,prot:u64)->Result<u64,i64> {
    let end=start.checked_add(length).ok_or(-12i64)?;
    if start<USER_BASE || start%PAGE!=0 || end>LIMIT || length>128*1024*1024 {return Err(-12);}
    let flags=if prot&2!=0 {Flags::WRITABLE}else{Flags::empty()} | if prot&4==0 {Flags::NO_EXECUTE}else{Flags::empty()};
    let old_len=p.pages.len();
    for v in (start..end).step_by(PAGE as usize) {
        if p.page(v,flags).is_err() {
            unmap(p,start,((p.pages.len()-old_len)as u64)*PAGE)?;
            return Err(-12);
        }
    }
    Ok(start)
}
fn unmap(p:&mut Program,start:u64,length:u64)->Result<i64,i64> {
    if start%PAGE!=0 {return Err(-22);}
    let end=start.checked_add(length).ok_or(-22i64)?;
    let root=unsafe{&mut *(p.root.start_address().as_u64() as *mut PageTable)};
    let mut mapper=unsafe{OffsetPageTable::new(root,VirtAddr::zero())};
    let mut i=0;
    while i<p.pages.len() {
        let (v,frame,_)=p.pages[i];
        if v>=start && v<end {
            mapper.unmap(Page::<Size4KiB>::containing_address(VirtAddr::new(v))).map_err(|_|-22i64)?.1.ignore();
            p.pages.swap_remove(i);
            if let Some(j)=p.frames.0.iter().position(|f|*f==frame){p.frames.0.swap_remove(j);phys::deallocate_frame(frame);}
        }else{i+=1;}
    }
    if Cr3::read().0==p.root {unsafe{Cr3::write(p.root,Cr3::read().1);}}
    Ok(0)
}
fn vectors(p:&Program,address:u64)->Result<Vec<String>,i64>{
    let mut result=Vec::new(); let mut size=0;
    if address==0{return Ok(result);}
    for i in 0..256 {let ptr=read64(p,address+i*8)?;if ptr==0{return Ok(result);}let s=p.path(ptr)?;size+=s.len()+1;if size>64*1024{return Err(-7);}result.push(s);}
    Err(-7)
}
unsafe fn fork(batch:*mut Batch,index:usize)->Result<i64,i64>{
    let slot=(*batch).programs.iter().position(|p|p.reaped);
    if slot.is_none() && (*batch).programs.len()>=64{return Err(-11);}
    let parent=&(*batch).programs[index];
    let mut child=Program::new().map_err(|_|-12i64)?;
    for (v,source,flags) in &parent.pages {
        let frame=child.page(*v,*flags).map_err(|_|-12i64)?;
        ptr::copy_nonoverlapping(source.start_address().as_u64() as *const u8,frame.start_address().as_u64() as *mut u8,PAGE as usize);
    }
    child.descriptors=parent.descriptors.clone();child.context=parent.context;child.context.registers[14]=0;
    child.linux=parent.linux.clone();child.parent=parent.pid;
    child.fpu.0.copy_from_slice(&parent.fpu.0);
    child.pid=(*batch).next_pid;let pid=child.pid;(*batch).next_pid+=1;
    if let Some(slot)=slot{(*batch).programs[slot]=child;}else{(*batch).programs.push(child);}
    Ok(pid as i64)
}
unsafe fn exec(batch:*mut Batch,index:usize,path:u64,args:u64,env:u64)->Result<i64,i64>{
    let p=&(*batch).programs[index];let path=path_at(p,-100i64 as u64,path)?;
    let mut argv=vectors(p,args)?;let env=vectors(p,env)?;
    let mut image=VFS.lock().read_file(&path).map_err(error)?;
    let mut executable_path=path.clone();
    if image.starts_with(b"#!") {
        let end=image.iter().position(|b|*b==b'\n').unwrap_or(image.len()).min(256);
        let header=core::str::from_utf8(&image[2..end]).map_err(|_|-8i64)?;
        let mut parts=header.split_whitespace();let interpreter=parts.next().ok_or(-8i64)?;
        if !interpreter.starts_with('/') {return Err(-8);}
        let mut script_args=vec![String::from(interpreter)];
        script_args.extend(parts.map(String::from));script_args.push(path.clone());
        script_args.extend(argv.into_iter().skip(1));argv=script_args;
        executable_path=String::from(interpreter);
        image=VFS.lock().read_file(&executable_path).map_err(error)?;
    }
    let elf=Executable::parse(&image).map_err(|_|-8i64)?;
    if !elf.linux {return Err(-8);}
    let mut next=load_program(&image).map_err(|e|if e=="out of memory"{-12i64}else{-8i64})?;
    initialize(&mut next,&elf,&argv,&env).map_err(|_|-7i64)?;
    next.linux.executable=executable_path;
    next.pid=p.pid;next.parent=p.parent;next.descriptors=p.descriptors.clone();
    next.linux.cwd=p.linux.cwd.clone();next.linux.paths=p.linux.paths.clone();next.linux.pgid=p.linux.pgid;
    next.linux.mask=p.linux.mask;next.linux.umask=p.linux.umask;
    for i in 0..64 {if p.linux.cloexec&(1<<i)!=0{next.descriptors[i]=None;next.linux.paths[i]=None;}}
    Cr3::write((*batch).kernel_root,Cr3::read().1);
    (*batch).programs[index]=next;
    Ok(0)
}
unsafe fn wait(batch:*mut Batch,index:usize,pid:u64,status:u64,flags:u64)->Result<i64,i64>{
    let p=&(*batch).programs[index]; if status!=0{p.validate_write(status,4)?;}
    let wanted=if pid as i64<=0{u64::MAX}else{pid};
    let parent=p.pid;
    let zombie=(*batch).programs.iter().find(|p|!p.reaped&&p.parent==parent&&(wanted==u64::MAX||p.pid==wanted)&&p.status.is_some());
    let encoded=zombie.map(|p|{let s=p.status.unwrap();if s<0{11u32}else{(s as u32&255)<<8}});
    let result=wait_child(batch,index,wanted,0,flags&1)?;
    if result>0&&status!=0{(*batch).programs[index].write_user(status,&encoded.unwrap().to_le_bytes())?;}
    Ok(result)
}
pub(super) unsafe fn call(batch:*mut Batch,index:usize,nr:u64,a:u64,b:u64,c:u64)->i64 {
    let result=(||->Result<i64,i64>{
        if nr==57 || nr==58 {return fork(batch,index);}
        if nr==56 {
            let flags=a;
            // Independent processes only; thread/shared-address-space clone is unsupported.
            if flags & !0x012000ff != 0 {return Err(-38);}
            return fork(batch,index);
        }
        if nr==59{return exec(batch,index,a,b,c);}
        if nr==61{return wait(batch,index,a,b,c);}
        let p=&mut (*batch).programs[index];
        let r=p.context.registers;
        let (d,e,f)=(r[5],r[7],r[6]); // r10,r8,r9
        match nr {
            0|1=>p.file_call(nr,a,b,c.min(4096)),
            2=>open(p,-100i64 as u64,a,b,c),
            257=>open(p,a,b,c,d),
            3=>{let result=p.file_call(3,a,0,0)?;p.linux.paths[a as usize]=None;p.linux.cloexec&=!(1<<a);Ok(result)},
            4|6|262=>{
                let (path,out,flags)=if nr==262 {(path_at(p,a,b)?,c,d)}else{(path_at(p,-100i64 as u64,a)?,b,if nr==6{256}else{0})};
                let stat=if flags&256!=0 {VFS.lock().lstat(&path)}else{VFS.lock().stat(&path)}.map_err(error)?;
                p.write_user(out,&stat_bytes(&stat))?;Ok(0)
            },
            5=>{let stat=stat_fd(p,a)?;p.write_user(b,&stat_bytes(&stat))?;Ok(0)},
            8=>p.file_call(8,a,b,c),
            9=>{
                if b==0{return Err(-22);}
                let len=b.checked_add(PAGE-1).ok_or(-12i64)?&!(PAGE-1);
                let start=if d&0x10!=0 {a}else{p.linux.mmap};
                if d&0x10!=0{unmap(p,start,len)?;}
                map(p,start,len,c|2)?;
                if d&0x20==0 {
                    let fd=p.descriptor(e)?;
                    let old=fd.seek(0,1)?;fd.seek(f as i64,0)?;
                    let mut buffer=[0u8;4096];let mut offset=0;
                    while offset<b {
                        let amount=((b-offset)as usize).min(4096);
                        let n=fd.read(&mut buffer[..amount])?;if n==0{break;}
                        p.write_user(start+offset,&buffer[..n])?;offset+=n as u64;
                    }
                    fd.seek(old,0)?;
                }
                p.linux.mmap=p.linux.mmap.max(start+len+PAGE);
                Ok(start as i64)
            },
            10=>{
                let end=a.checked_add(b).ok_or(-22i64)?;
                for (v,_,flags) in &mut p.pages {if *v>=a&&*v<end {
                    flags.set(Flags::WRITABLE,c&2!=0);flags.set(Flags::NO_EXECUTE,c&4==0);
                    let root=&mut *(p.root.start_address().as_u64() as *mut PageTable);
                    OffsetPageTable::new(root,VirtAddr::zero()).update_flags(Page::<Size4KiB>::containing_address(VirtAddr::new(*v)),*flags|Flags::PRESENT|Flags::USER_ACCESSIBLE).map_err(|_|-22i64)?.ignore();
                }}
                if Cr3::read().0==p.root{Cr3::write(p.root,Cr3::read().1);}Ok(0)
            },
            11=>unmap(p,a,b),
            12=>{
                if a==0{return Ok(p.linux.brk as i64);}
                if a<p.linux.heap_start||a>=MMAP_START{return Ok(p.linux.brk as i64);}
                let old=(p.linux.brk+PAGE-1)&!(PAGE-1);let end=(a+PAGE-1)&!(PAGE-1);
                if end>old&&map(p,old,end-old,3).is_err(){return Ok(p.linux.brk as i64);}
                if end<old{unmap(p,end,old-end)?;}
                p.linux.brk=a;Ok(a as i64)
            },
            13=>{if a==0||a>64||a==9||a==19||d!=8{return Err(-22);}if c!=0{p.write_user(c,&p.linux.actions[a as usize])?;}if b!=0{let mut action=[0;32];p.read_user(b,&mut action).map_err(|_|-14i64)?;p.linux.actions[a as usize]=action;}Ok(0)},
            14=>{if d!=8{return Err(-22);}if c!=0{p.write_user(c,&p.linux.mask.to_le_bytes())?;}if b!=0 {let value=read64(p,b)?;match a{0=>p.linux.mask|=value,1=>p.linux.mask&=!value,2=>p.linux.mask=value,_=>return Err(-22)}}Ok(0)},
            16=>ioctl(p,a,b,c),
            17|18=>{
                let fd=p.descriptor(a)?;let pos=fd.seek(0,1)?;fd.seek(d as i64,0)?;
                let result=p.file_call(if nr==17{0}else{1},a,b,c.min(4096));let _=fd.seek(pos,0);result
            },
            19|20=>{
                if c>1024{return Err(-22);}let mut total=0;
                for i in 0..c {let addr=read64(p,b+i*16)?;let len=read64(p,b+i*16+8)?;let mut offset=0;
                    while offset<len{match p.file_call(if nr==19{0}else{1},a,addr+offset,(len-offset).min(4096)){
                        Ok(n)=>{total+=n;offset+=n as u64;if n==0||nr==19{return Ok(total);}},
                        Err(err)=>return if total==0{Err(err)}else{Ok(total)},
                    }}
                }Ok(total)
            },
            21|269|439=>{let name=if nr==21{path_at(p,-100i64 as u64,a)?}else{path_at(p,a,b)?};VFS.lock().stat(&name).map_err(error)?;Ok(0)},
            22|293=>{let result=p.file_call(22,a,0,0)?;if nr==293&&b&0x80000!=0{let x=read32(p,a)?;let y=read32(p,a+4)?;p.linux.cloexec|=(1<<x)|(1<<y);}Ok(result)},
            24=>Ok(0),
            28=>Ok(0), // advisory madvise; eager private mappings
            32=>{let slot=fd_slot(p,0)?;duplicate(p,a,slot,false)},
            33=>duplicate(p,a,b as usize,false),
            292=>{if a==b{return Err(-22);}duplicate(p,a,b as usize,c&0x80000!=0)},
            35|230=>{
                let request=if nr==35{a}else{c};
                let secs=read64(p,request)?;let nanos=read64(p,request+8)?;
                if nanos>=1_000_000_000{return Err(-22);}
                let duration=secs.checked_mul(1000).and_then(|v|v.checked_add((nanos+999_999)/1_000_000)).ok_or(-22i64)?;
                let now=crate::boottime::elapsed_ms();
                let target=if nr==230&&b&1!=0{duration}else{now.checked_add(duration).ok_or(-22i64)?};
                let deadline=*p.linux.poll_deadline.get_or_insert(target);
                if now<deadline{return Err(-11);}
                p.linux.poll_deadline=None;Ok(0)
            },
            39|186=>Ok(p.pid as i64),
            63=>{let mut data=[0u8;390];for (i,s) in ["bOS","bos","0.6.0","bOS Linux ABI","x86_64","localdomain"].iter().enumerate(){data[i*65..i*65+s.len()].copy_from_slice(s.as_bytes());}p.write_user(a,&data)?;Ok(0)},
            72=>{
                p.descriptor(a)?;
                match b {
                    0|1030=>{let slot=fd_slot(p,c as usize)?;duplicate(p,a,slot,b==1030)},
                    1=>Ok(((p.linux.cloexec>>a)&1)as i64),
                    2=>{p.linux.cloexec&=!(1<<a);if c&1!=0{p.linux.cloexec|=1<<a;}Ok(0)},
                    3=>Ok(match &*p.descriptor(a)?{Handle::ConsoleWrite|Handle::PipeWrite(_)=>1,_=>0}),
                    4=>Ok(0), _=>Err(-22)
                }
            },
            73=>Ok(0), // flock: single-user BSP; VFS writes serialized
            74|75|162=>p.file_call(if nr==75{74}else{nr},a,b,c),
            76|77=>{let path=if nr==76{path_at(p,-100i64 as u64,a)?}else{p.linux.paths.get(a as usize).and_then(Clone::clone).ok_or(-9i64)?};VFS.lock().truncate(&path,b).map_err(error)?;Ok(0)},
            79=>{let bytes=p.linux.cwd.as_bytes();if b<bytes.len()as u64+1{return Err(-34);}p.write_user(a,bytes)?;p.write_user(a+bytes.len()as u64,&[0])?;Ok(bytes.len()as i64+1)},
            80|81=>{let path=if nr==80{path_at(p,-100i64 as u64,a)?}else{p.linux.paths.get(a as usize).and_then(Clone::clone).ok_or(-9i64)?};let stat=VFS.lock().stat(&path).map_err(error)?;if !stat.is_dir(){return Err(-20);}p.linux.cwd=path;Ok(0)},
            82|264=>{let (from,to)=if nr==82{(path_at(p,-100i64 as u64,a)?,path_at(p,-100i64 as u64,b)?)}else{(path_at(p,a,b)?,path_at(p,c,d)?)};VFS.lock().rename(&from,&to).map_err(error)?;Ok(0)},
            83|258=>{let (path,mode)=if nr==83{(path_at(p,-100i64 as u64,a)?,b)}else{(path_at(p,a,b)?,c)};let vfs=VFS.lock();vfs.mkdir(&path).map_err(error)?;let _=vfs.chmod(&path,mode as u32&0o777&!p.linux.umask);Ok(0)},
            84|87|263=>{let path=if nr==263{path_at(p,a,b)?}else{path_at(p,-100i64 as u64,a)?};VFS.lock().remove(&path).map_err(error)?;Ok(0)},
            88|266=>{let target=p.path(a)?;let link=if nr==88{path_at(p,-100i64 as u64,b)?}else{path_at(p,b,c)?};VFS.lock().symlink(&target,&link).map_err(error)?;Ok(0)},
            89|267=>{let (name,out,len)=if nr==89{(path_at(p,-100i64 as u64,a)?,b,c)}else{(path_at(p,a,b)?,c,d)};let target=if name=="/proc/self/exe"{p.linux.executable.clone()}
                    else if let Some(fd)=name.strip_prefix("/proc/self/fd/").and_then(|s|s.parse::<usize>().ok()){p.linux.paths.get(fd).and_then(Clone::clone).ok_or(-2i64)?}
                    else{VFS.lock().readlink(&name).map_err(error)?};let size=target.len().min(len as usize);p.write_user(out,&target.as_bytes()[..size])?;Ok(size as i64)},
            90|91|268|452=>{let (path,mode)=if nr==90{(path_at(p,-100i64 as u64,a)?,b)}else if nr==91{(p.linux.paths.get(a as usize).and_then(Clone::clone).ok_or(-9i64)?,b)}else{(path_at(p,a,b)?,c)};VFS.lock().chmod(&path,mode as u32).map_err(error)?;Ok(0)},
            95=>{let old=p.linux.umask;p.linux.umask=a as u32&0o777;Ok(old as i64)},
            96|228=>{let now=crate::boottime::elapsed_ms();let mut time=[0u8;16];time[..8].copy_from_slice(&(now/1000).to_le_bytes());time[8..].copy_from_slice(&((now%1000)*if nr==96{1000}else{1_000_000}).to_le_bytes());p.write_user(if nr==96{a}else{b},&time)?;Ok(0)},
            97|302=>{let (resource,out)=if nr==97{(a,b)}else{(b,d)};if nr==302&&a!=0&&a!=p.pid{return Err(-3);}if out!=0{let limit=match resource{3=>1024*1024,7=>64,6=>64,_=>u64::MAX};let mut bytes=[0u8;16];bytes[..8].copy_from_slice(&limit.to_le_bytes());bytes[8..].copy_from_slice(&limit.to_le_bytes());p.write_user(out,&bytes)?;}Ok(0)},
            98=>{p.write_user(b,&[0;144])?;Ok(0)},
            102|104|107|108=>Ok(0),
            115=>{if a>0{p.write_user(b,&0u32.to_le_bytes())?;}Ok(1)},
            204=>{if b<8{return Err(-22);}p.write_user(c,&1u64.to_le_bytes())?;Ok(8)},
            110=>Ok(p.parent as i64),
            111=>Ok(p.linux.pgid as i64),
            109=>{let pid=p.pid;let pg=if b==0{if a==0{pid}else{a}}else{b};let target=if a==0{pid}else{a};let q=(*batch).programs.iter_mut().find(|q|q.pid==target&&q.status.is_none()).ok_or(-3i64)?;q.linux.pgid=pg;Ok(0)},
            112=>{p.linux.pgid=p.pid;Ok(p.pid as i64)},
            121=>{let pid=if a==0{p.pid}else{a};Ok((*batch).programs.iter().find(|q|q.pid==pid).ok_or(-3i64)?.linux.pgid as i64)},
            131=>{if b!=0{p.write_user(b,&[0;24])?;}Ok(0)},
            158=>match a{0x1002=>{if b<USER_BASE||b>=LIMIT{return Err(-1);}p.linux.fs=b;Ok(0)},0x1003=>{p.write_user(b,&p.linux.fs.to_le_bytes())?;Ok(0)},_=>Err(-22)},
            202=>{if b&127==1{Ok(0)}else if read32(p,a)?!=c as u32{Err(-11)}else{Err(-38)}},
            217=>getdents(p,a,b,c),
            218=>Ok(p.pid as i64),
            273=>Ok(0),
            318=>{let size=b.min(4096)as usize;let mut bytes=[0;4096];random_bytes(&mut bytes[..size])?;p.write_user(a,&bytes[..size])?;Ok(size as i64)},
            7=>poll(p,a,b,c as i32),
            270=>{let timeout=if c==0{-1}else{let secs=read64(p,c)?;let ns=read64(p,c+8)?;(secs*1000+ns/1_000_000).min(i32::MAX as u64)as i32};poll(p,a,b,timeout)},
            62|200|234=>{
                let (pid,signal)=if nr==234{(b,c)}else{(a,b)};if signal>64{return Err(-22);}
                let pg=p.linux.pgid;
                let mut found=false;
                for i in 0..(*batch).programs.len(){let q=&(*batch).programs[i];
                    if q.status.is_none()&&(pid==q.pid||pid==0&&q.linux.pgid==pg||(pid as i64)< -1&&q.linux.pgid==(-(pid as i64))as u64){
                        found=true;if signal!=0&&!matches!(signal,17|18|28){terminate(batch,i,128+signal as i64);}
                    }
                }if found{Ok(0)}else{Err(-3)}
            },
            436=>{for fd in a..=b.min(63){p.descriptors[fd as usize]=None;p.linux.paths[fd as usize]=None;}Ok(0)},
            _=>{log::debug!("Linux syscall {} not implemented",nr);Err(-38)}
        }
    })();
    result.unwrap_or_else(|e|e)
}
fn getdents(p:&mut Program,fd:u64,out:u64,length:u64)->Result<i64,i64>{
    if !matches!(&*p.descriptor(fd)?,Handle::Directory){return Err(-20);}
    let path=p.linux.paths[fd as usize].as_ref().ok_or(-9i64)?;
    let mut entries=VFS.lock().list_dir(path).map_err(error)?;
    entries.insert(0,fs::DirEntry{inode:1,name:"..".into(),file_type:fs::FileType::Directory});
    entries.insert(0,fs::DirEntry{inode:1,name:".".into(),file_type:fs::FileType::Directory});
    let mut data=Vec::new();let mut cursor=p.linux.offsets[fd as usize];
    while cursor<entries.len(){let ent=&entries[cursor];let size=(19+ent.name.len()+1+7)&!7;
        if data.len()+size>length.min(65536)as usize{if data.is_empty(){return Err(-22);}break;}
        let offset=data.len();data.resize(offset+size,0);
        data[offset..offset+8].copy_from_slice(&ent.inode.to_le_bytes());
        data[offset+8..offset+16].copy_from_slice(&((cursor+1)as u64).to_le_bytes());
        data[offset+16..offset+18].copy_from_slice(&(size as u16).to_le_bytes());
        data[offset+18]=match ent.file_type{fs::FileType::Directory=>4,fs::FileType::Symlink=>10,_=>8};
        data[offset+19..offset+19+ent.name.len()].copy_from_slice(ent.name.as_bytes());cursor+=1;
    }
    p.write_user(out,&data)?;p.linux.offsets[fd as usize]=cursor;Ok(data.len()as i64)
}
fn ioctl(p:&mut Program,fd:u64,request:u64,arg:u64)->Result<i64,i64>{
    if !matches!(&*p.descriptor(fd)?,Handle::ConsoleRead|Handle::ConsoleWrite){return Err(-25);}
    match request {
        0x5401=>{p.write_user(arg,&p.linux.termios)?;Ok(0)},
        0x5402..=0x5404=>{let mut termios=[0;36];p.read_user(arg,&mut termios).map_err(|_|-14i64)?;p.linux.termios=termios;Ok(0)},
        0x5413=>{p.write_user(arg,&[25,0,80,0,0,0,0,0])?;Ok(0)},
        0x540f=>{p.write_user(arg,&(p.linux.pgid as u32).to_le_bytes())?;Ok(0)},
        0x5410=>Ok(0),
        _=>Err(-25)
    }
}
fn poll(p:&mut Program,fds:u64,count:u64,timeout:i32)->Result<i64,i64>{
    if count>64{return Err(-22);}
    let mut ready=0;
    for i in 0..count {
        let address=fds+i*8;let fd=read32(p,address)? as i32;let mut bytes=[0;2];p.read_user(address+4,&mut bytes).map_err(|_|-14i64)?;
        let events=u16::from_le_bytes(bytes);let mut revents=0u16;
        if fd>=0{match p.descriptor(fd as u64){
            Err(_)=>revents=32,
            Ok(handle)=>{
                if events&1!=0 && match &*handle{Handle::ConsoleRead=>unsafe{let b=BATCH.load(Ordering::Acquire);!b.is_null()&&(*b).input_len>0},Handle::PipeRead(pipe)=>{let pipe=pipe.lock();pipe.length>0||pipe.writers==0},_=>true}{revents|=1;}
                if events&4!=0 {revents|=4;}
            }
        }}
        p.write_user(address+6,&revents.to_le_bytes())?;if revents!=0{ready+=1;}
    }
    if ready>0||timeout==0{p.linux.poll_deadline=None;return Ok(ready);}
    let now=crate::boottime::elapsed_ms();
    if timeout>0 {let deadline=*p.linux.poll_deadline.get_or_insert(now+timeout as u64);if now>=deadline{p.linux.poll_deadline=None;return Ok(0);}}
    Err(-11)
}
