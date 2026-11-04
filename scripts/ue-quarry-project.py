"""Prepare the existing private project's cook defaults; does not launch Unreal."""
import hashlib
import json
from pathlib import Path
import re
import sys

ROOT = Path("D:/Coding/renderlab-work/ue-m5")


def update(text, section, key, value):
    start = text.index("[" + section + "]")
    end = text.find("\n[", start + 1)
    end = len(text) if end < 0 else end
    block = text[start:end]
    pattern = re.compile(r"(?m)^" + re.escape(key) + r"=.*$")
    line = key + "=" + value
    block = pattern.sub(lambda _: line, block) if pattern.search(block) else block.rstrip() + "\n" + line + "\n"
    return text[:start] + block + text[end:]


def prepare():
    path = ROOT / "Config/DefaultEngine.ini"
    before = path.read_bytes()
    backup = ROOT / "Quarry/config-before-full-detail.ini"
    if backup.exists():
        raise ValueError("project cook defaults were already prepared")
    backup.write_bytes(before)
    text = before.decode("utf-8-sig").replace("\r\n", "\n")
    values = {
        "/Script/EngineSettings.GameMapsSettings": {"GameDefaultMap": "/Game/M6Quarry/View0"},
        "/Script/Engine.RendererSettings": {"r.RayTracing": "True", "r.SkinCache.CompileShaders": "True", "r.DefaultFeature.AutoExposure": "True"},
        "SystemSettings": {"r.ShadowQuality": "5", "r.Shadow.Virtual.Enable": "1", "r.EyeAdaptationQuality": "2",
            "r.EyeAdaptation.MethodOverride": "3", "r.EyeAdaptation.LensAttenuation": "0.78", "r.RayTracing.Nanite.Mode": "1",
            "r.Nanite.MaxPixelsPerEdge": "0.25", "r.MaxAnisotropy": "16"}}
    for section, entries in values.items():
        for key, value in entries.items():
            text = update(text, section, key, value)
    text = re.sub(r"(?m)^r.UsePreExposure=.*\n?", "", text)
    path.write_text(text)
    (ROOT / "Quarry/project-cook-defaults.json").write_text(json.dumps(dict(settings=values,
        before_sha256=hashlib.sha256(before).hexdigest(), after_sha256=hashlib.sha256(path.read_bytes()).hexdigest()), indent=2) + "\n")


def content_only():
    path = ROOT / "M5Calibration.uproject"
    before = path.read_bytes()
    value = json.loads(before)
    for plugin in value["Plugins"]:
        if plugin["Name"] == "GeometryScripting":
            plugin["Enabled"] = False
    path.write_text(json.dumps(value, indent=2) + "\n")
    (ROOT / "Quarry/content-only-plugin.json").write_text(json.dumps(dict(
        before_sha256=hashlib.sha256(before).hexdigest(), after_sha256=hashlib.sha256(path.read_bytes()).hexdigest(),
        disabled="GeometryScripting", reason="setup finished; quarry maps do not use it; plugin otherwise requires a compiled game target"), indent=2) + "\n")


def sm6():
    path = ROOT / "Config/DefaultEngine.ini"
    text = path.read_text()
    section = "/Script/WindowsTargetPlatform.WindowsTargetSettings"
    if "[" + section + "]" in text:
        raise ValueError("Windows target settings already exist; inspect before changing")
    text += ("\n[" + section + "]\nDefaultGraphicsRHI=DefaultGraphicsRHI_DX12\n"
             "-D3D12TargetedShaderFormats=PCD3D_SM5\n+D3D12TargetedShaderFormats=PCD3D_SM6\n")
    path.write_text(text)
    (ROOT / "Quarry/sm6-cook-settings.json").write_text(json.dumps(dict(section=section,
        shader_format="PCD3D_SM6", ini_sha256=hashlib.sha256(path.read_bytes()).hexdigest()), indent=2) + "\n")


if __name__ == "__main__":
    if sys.argv[1:] == ["--content-only"]:
        content_only()
    elif sys.argv[1:] == ["--sm6"]:
        sm6()
    else:
        prepare()
