"""Offline f64 GGX / correlated Smith / Schlick DFG. No third-party source.

128² endpoints (NoV >= 1e-4), 65536 Hammersley samples, RG16F little endian.
This generator establishes reproducibility; independent quadrature tests establish accuracy.
"""
from pathlib import Path
import numpy as np


def integrate(nov, roughness, samples=65536):
    i = np.arange(samples, dtype=np.uint32)
    bits = i.copy()
    bits = (bits << 16) | (bits >> 16)
    bits = ((bits & 0x55555555) << 1) | ((bits & 0xAAAAAAAA) >> 1)
    bits = ((bits & 0x33333333) << 2) | ((bits & 0xCCCCCCCC) >> 2)
    bits = ((bits & 0x0F0F0F0F) << 4) | ((bits & 0xF0F0F0F0) >> 4)
    bits = ((bits & 0x00FF00FF) << 8) | ((bits & 0xFF00FF00) >> 8)
    u = bits.astype(np.float64) / 4294967296.0
    jacobian = 3*(1-u)**2
    u = 1-(1-u)**3
    phi = i.astype(np.float64) * (2*np.pi/samples)
    alpha = max(roughness*roughness, 1e-4)
    hz = np.sqrt((1-u) / ((1-u)+alpha*alpha*u))
    hx = np.sqrt(np.maximum(0, 1-hz*hz))*np.cos(phi)
    vh = np.maximum(np.sqrt(1-nov*nov)*hx+nov*hz, 0)
    nl = np.maximum(2*vh*hz-nov, 0)
    visibility = .5 / np.maximum(nl*np.sqrt(nov*nov*(1-alpha*alpha)+alpha*alpha) + nov*np.sqrt(nl*nl*(1-alpha*alpha)+alpha*alpha), 1e-30)
    weight = 4*visibility*nl*vh / np.maximum(hz, 1e-30)*jacobian
    fc = (1-vh)**5
    return np.array([np.mean(weight*(1-fc)), np.mean(weight*fc)])


if __name__ == '__main__':
    size = 128
    data = np.empty((size, size, 2), dtype='<f2')
    for y in range(size):
        for x in range(size):
            data[y, x] = integrate(max(x/(size-1), 1e-4), y/(size-1))
    path = Path(__file__).resolve().parents[1] / 'src/dfg.rg16f'
    path.write_bytes(data.tobytes())
    print(path, path.stat().st_size)
