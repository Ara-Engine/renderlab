// Diagnostic sidecar: epoch, last own gather, last parent copy, parent slot.
@group(0) @binding(38) var<storage,read_write> gp_cells:array<u32>;
fn gp_reset(slot:u32){let a=16u+4u*slot;gp_cells[a]=gp_cells[0];gp_cells[a+1u]=0u;gp_cells[a+2u]=0u;gp_cells[a+3u]=0xffffffffu;}
fn gp_parent(slot:u32,parent:u32){let a=16u+4u*slot;gp_cells[a+2u]=gp_cells[0];gp_cells[a+3u]=parent;}
fn gp_gather(slot:u32){gp_cells[16u+4u*slot+1u]=gp_cells[0];}
