use bytes::{Buf, BufMut, Bytes, BytesMut};
use mimalloc::MiMalloc;
use std::collections::BTreeMap;
use std::fs::File;
use std::io::Write;
use std::path::Path;
use std::time::Instant;

#[global_allocator]
static GLOBAL: MiMalloc = MiMalloc;

const POLYS: [u32; 32] = [
    0x0001D258, 0x00017E04, 0x0001FF6B, 0x00013F67, 0x0001B9EE, 0x000198D1, 0x000178C7, 0x00018A55,
    0x00015777, 0x0001D911, 0x00015769, 0x0001991F, 0x00012BD0, 0x0001CF73, 0x0001365D, 0x000197F5,
    0x000194A0, 0x0001B279, 0x00013A34, 0x0001AE41, 0x000180D4, 0x00017891, 0x00012E64, 0x00017C72,
    0x00019C6D, 0x00013F32, 0x0001AE14, 0x00014E76, 0x00013C97, 0x000130CB, 0x00013750, 0x0001CB8D,
];

fn lfsr_gen(poly: u32) -> impl FnMut() -> Option<(u32, u32)> {
    let mut state = 1;
    let mut step = 0;

    move || {
        if step >= 1 << 17 {
            None
        } else {
            let old = (step, state);
            state = (state << 1) | ((state & poly).count_ones() & 1);
            step += 1;
            Some(old)
        }
    }
}

struct Lookup<'a>(&'a [u8], usize);

impl<'a> Lookup<'a> {
    pub fn find(&self, item: u32) -> Option<(usize, u32)> {
        let li = (item & ((1 << 17) - 1)) as usize;
        let item = (item & ((1 << 25) - 1));
        if li > self.1 {
            return None;
        }

        let ptr = (&self.0[li..li + 4]).get_u32();
        let len = ptr >> 19;
        let ptr = ptr & ((1 << 19) - 1);

        let mut window = &self.0[ptr as usize..(ptr + len) as usize];
        for _ in 0..len {
            let (k, v) = (window.get_u32(), window.get_u32());
            if k == item {
                let ch = v >> 17;
                let arc = v & ((1 << 17) - 1);
                return Some((ch as usize, arc));
            }
        }

        None
    }
}

#[allow(dead_code)]
fn main() {
    let mut gen = lfsr_gen(POLYS[10]);
    let (step, state) = (0..10_000).filter_map(|_| gen()).last().unwrap();

    let mut lookup = Bytes::from(&include_bytes!("../lookup.bin")[..]);
    let len = lookup.get_u32();
    let lookup = Lookup(lookup.chunk(), len as usize);

    println!("{:?}", lookup.find(state & ((1 << 25) - 1)));
}

#[allow(dead_code)]
fn _main() {
    let mut btree = BTreeMap::new();
    for (i, poly) in POLYS.into_iter().enumerate() {
        let mut lfsr = lfsr_gen(poly);
        while let Some((arc, state)) = lfsr() {
            btree
                .entry(state & ((1 << 19) - 1))
                .or_insert(Vec::new())
                .push((state & ((1 << 25) - 1), arc | ((i as u32) << 17)));
        }
    }
    eprintln!("made btree, size={}", btree.len());

    let mut last = 0;
    let mut lookup_size = 0u32;
    for (&k, _) in &btree {
        let diff = k - last;
        lookup_size += diff;
        last = k;
    }
    lookup_size *= 3;

    let mut buf = BytesMut::with_capacity(1_000_000_000);
    let mut lookup = BytesMut::with_capacity(1 << 22);
    let mut file = File::create("lookup.bin").unwrap();
    eprintln!("index size: {}", lookup_size);

    file.write_all(&(btree.len() as u32).to_be_bytes()).unwrap();
    eprintln!("made file and buffers");

    let len = btree.len();
    let mut timer = Instant::now();
    let mut total = 0.0;
    let mut last = 0;
    for (i, (k, v)) in btree.into_iter().enumerate() {
        if lookup.len() + 3 > lookup.capacity() {
            file.write_all(&lookup).unwrap();
            lookup.clear();
        }

        let diff = k - last - 1;
        lookup.put_bytes(0, (diff * 3) as usize);
        last = k;

        let index = (v.len() << 19) as u32 | (buf.len() as u32 + lookup_size as u32);
        lookup.put_slice(&[(index >> 16) as u8, (index >> 8) as u8, index as u8]);

        for (s, a) in v {
            buf.put_u32(s);
            buf.put_u32(a);
        }

        if timer.elapsed().as_secs_f64() > 1.0 {
            total += timer.elapsed().as_secs_f64();
            eprintln!("[{total:.0}s | {i}/{}]: {k} {len}", buf.len());
            timer = Instant::now();
        }
    }

    eprintln!("writing files");
    eprintln!("writing lookup");
    file.write_all(&lookup).unwrap();
    eprintln!("writing buffer");
    file.write_all(&buf).unwrap();
    file.flush().unwrap();
}
