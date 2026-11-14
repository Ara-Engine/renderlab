//! Diagnostic-only snapshots after pixel probing, before any gather can stamp a hit cell.
use super::*;

pub(super) fn enabled() -> bool { std::env::var_os("RENDERLAB_GI_QUEUE_DIAG").is_some() }
pub(super) struct State {
    buffer: wgpu::Buffer,
    table_words: usize,
    raw_words: usize,
    cold_words: usize,
    pending: bool,
}
impl State {
    pub fn new(device: &wgpu::Device, r: &Resources) -> Self {
        let size=r.table.size()+r.raw.size()+u64::from(r.slots+1)*4;
        Self {buffer:device.create_buffer(&wgpu::BufferDescriptor {
            label:Some("GI queue groups diagnostic snapshot"),size,
            usage:wgpu::BufferUsages::COPY_DST|wgpu::BufferUsages::MAP_READ,mapped_at_creation:false,
        }),table_words:r.table.size() as usize/4,raw_words:r.raw.size() as usize/4,
        cold_words:r.cold_words as usize,pending:true}
    }
    pub fn arm(&mut self) {self.pending=true;}
    pub fn encode(&mut self, encoder:&mut wgpu::CommandEncoder, r:&Resources, queue:u32) {
        if !self.pending {return;}
        encoder.copy_buffer_to_buffer(&r.table,0,&self.buffer,0,r.table.size());
        encoder.copy_buffer_to_buffer(&r.raw,0,&self.buffer,r.table.size(),r.raw.size());
        encoder.copy_buffer_to_buffer(&r.work,u64::from(queue)*4,&self.buffer,r.table.size()+r.raw.size(),u64::from(r.slots+1)*4);
        self.pending=false;
    }
    pub fn read(&self, device:&wgpu::Device) -> Result<[u32;7],Error> {
        if self.pending {return Err(Error::InvalidInput("GI queue probe did not run"));}
        let slice=self.buffer.slice(..);slice.map_async(wgpu::MapMode::Read, |_| {});
        device.poll(WAIT).map_err(Error::Poll)?;
        let mapped=slice.get_mapped_range().map_err(|_|Error::InvalidInput("GI queue probe map"))?;
        let words:Vec<u32>=mapped.chunks_exact(4).map(|b|u32::from_le_bytes(b.try_into().unwrap())).collect();
        let result=classify(&words,self.table_words,self.raw_words,self.cold_words);
        drop(mapped);self.buffer.unmap();Ok(result)
    }
}
/// First visible / refresh visible / background / ineligible / duplicate / entries /
/// visible first-fill cells that already have filtered neighbour data.
fn classify(words:&[u32], table:usize, raw:usize, stride:usize) -> [u32;7] {
    let slots=words[1] as usize+1;let frame=words[3];let queue=table+raw;
    let count=(words[queue] as usize).min(slots);let mut counts=[0u32;7];counts[5]=count as u32;
    let mut seen=vec![false;slots];
    for &item in &words[queue+1..queue+1+count] {
        let slot=item as usize;
        if slot>=slots {counts[3]+=1;continue;}
        if std::mem::replace(&mut seen[slot],true) {counts[4]+=1;continue;}
        let hot=HEADER_WORDS as usize+slot*24;let cold=HEADER_WORDS as usize+slots*24+slot*stride;
        let state=words[cold+1];
        if words[hot]==0 || state&128==0 || state&7!=1 {counts[3]+=1;continue;}
        if words[cold+2]!=frame {counts[2]+=1;continue;}
        let value=if words[24]&2!=0 {words[table+slot*RAW_WORDS as usize+15]} else {words[hot+16]};
        if value==0 {counts[0]+=1;if words[hot+16]!=0 {counts[6]+=1;}}
        else {counts[1]+=1;}
    }
    counts
}
#[cfg(test)] mod tests {
    use super::*;
    #[test] fn raw_zero_and_gathered_zero_remain_distinct_after_filtering() {
        let slots=4;let stride=24;let table=64+slots*(24+stride);let raw=slots*RAW_WORDS as usize;
        let mut w=vec![0;table+raw+slots+1];w[1]=3;w[3]=7;w[24]=2;
        for slot in 0..3 {let hot=64+slot*24;let cold=64+slots*24+slot*stride;w[hot]=1;w[hot+16]=123;w[cold+1]=129;w[cold+2]=if slot==2 {6}else{7};}
        w[table+RAW_WORDS as usize+15]=0x80000000; // A gathered zero is data.
        w[table+raw]=4;w[table+raw+1..].copy_from_slice(&[0,1,2,0]);
        assert_eq!(classify(&w,table,raw,stride),[1,1,1,0,1,4,1]);
        assert!(include_str!("../shaders/gi_cache.wgsl").contains("const GI_RAW_WORDS:u32=22u;"));
    }
}
