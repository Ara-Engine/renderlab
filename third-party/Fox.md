# Fox test asset

SK1's test captures and `examples/skinned-mesh.rs` use **Fox**, from the Khronos
glTF Sample Assets collection. The repository does not bundle the GLB; fetch the
pinned fixture with `py -3 scripts/m7-sk1-asset.py`.

- Source: <https://github.com/KhronosGroup/glTF-Sample-Assets/tree/8f492390ddc7a2ae221a1369ef560589cf8c70fb/Models/Fox>
- Original model: PixelMannen, © 2014 Public, [CC0 1.0](https://creativecommons.org/publicdomain/zero/1.0/legalcode).
- Rigging and animation: tomkranis, © 2014, [CC BY 4.0](https://creativecommons.org/licenses/by/4.0/legalcode).
- Conversion to glTF: @AsoboStudio and @scurest, © 2017, [CC BY 4.0](https://creativecommons.org/licenses/by/4.0/legalcode).
- GLB SHA-256: `d97044e701822bac5a62696459b27d7b375aada5de8574ed4362edbba94771f7`.

The source model is unchanged. Renderlab cooks its geometry, skin and animations
into its own RLS format, forces double-sided rendering for the current hardware
shadow contract, scales the fox and adds example movement. Capture images are
derived renders and retain this attribution. The quarry is a separate local asset
and is not redistributed.
