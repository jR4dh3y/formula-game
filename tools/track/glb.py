"""Small glTF 2.0 binary writer for game-ready static geometry."""

from __future__ import annotations

import io
import json
import struct
from dataclasses import dataclass

import numpy as np
from PIL import Image


@dataclass
class GlbMaterial:
    name: str
    base_color: tuple[float, float, float, float] = (1.0, 1.0, 1.0, 1.0)
    image: Image.Image | None = None
    alpha_mask: bool = False
    roughness: float = 0.8
    metallic: float = 0.0
    emissive: tuple[float, float, float] = (0.0, 0.0, 0.0)


@dataclass
class GlbMesh:
    name: str
    material: int
    positions: np.ndarray  # (n, 3) float32
    normals: np.ndarray  # (n, 3) float32
    uvs: np.ndarray  # (n, 2) float32
    indices: np.ndarray  # (t * 3,) uint32


class _Buffer:
    def __init__(self):
        self.data = bytearray()
        self.views: list[dict] = []

    def add(self, payload: bytes, target: int | None = None) -> int:
        while len(self.data) % 4:
            self.data.append(0)
        view = {"buffer": 0, "byteOffset": len(self.data), "byteLength": len(payload)}
        if target is not None:
            view["target"] = target
        self.data.extend(payload)
        self.views.append(view)
        return len(self.views) - 1


def encode_image(image: Image.Image, max_dimension: int) -> tuple[bytes, str]:
    largest = max(image.size)
    if largest > max_dimension:
        scale = max_dimension / largest
        image = image.resize(
            (max(1, round(image.width * scale)), max(1, round(image.height * scale))),
            Image.LANCZOS,
        )
    out = io.BytesIO()
    if image.mode == "RGBA":
        image.save(out, "PNG", optimize=True)
        return out.getvalue(), "image/png"
    image.convert("RGB").save(out, "JPEG", quality=90)
    return out.getvalue(), "image/jpeg"


def write_glb(
    path: str,
    materials: list[GlbMaterial],
    meshes: list[GlbMesh],
    max_texture_dimension: int = 1024,
) -> None:
    buffer = _Buffer()
    accessors: list[dict] = []
    gltf: dict = {
        "asset": {"version": "2.0", "generator": "f1-the-game track tools"},
        "scene": 0,
        "scenes": [{"nodes": list(range(len(meshes)))}],
        "nodes": [],
        "meshes": [],
        "materials": [],
        "samplers": [{"magFilter": 9729, "minFilter": 9987, "wrapS": 10497, "wrapT": 10497}],
        "textures": [],
        "images": [],
        "accessors": accessors,
        "bufferViews": buffer.views,
    }

    for material in materials:
        entry = {
            "name": material.name,
            "pbrMetallicRoughness": {
                "baseColorFactor": list(material.base_color),
                "metallicFactor": material.metallic,
                "roughnessFactor": material.roughness,
            },
            "doubleSided": True,
        }
        if material.emissive != (0.0, 0.0, 0.0):
            entry["emissiveFactor"] = list(material.emissive)
        if material.image is not None:
            payload, mime = encode_image(material.image, max_texture_dimension)
            gltf["images"].append({"bufferView": buffer.add(payload), "mimeType": mime})
            gltf["textures"].append({"sampler": 0, "source": len(gltf["images"]) - 1})
            entry["pbrMetallicRoughness"]["baseColorTexture"] = {
                "index": len(gltf["textures"]) - 1
            }
        if material.alpha_mask:
            entry["alphaMode"] = "MASK"
            entry["alphaCutoff"] = 0.5
        gltf["materials"].append(entry)

    def accessor(array: np.ndarray, kind: str, component: int, target: int, bounds=False) -> int:
        view = buffer.add(array.tobytes(), target)
        entry = {
            "bufferView": view,
            "componentType": component,
            "count": int(array.shape[0]),
            "type": kind,
        }
        if bounds:
            entry["min"] = array.min(axis=0).tolist()
            entry["max"] = array.max(axis=0).tolist()
        accessors.append(entry)
        return len(accessors) - 1

    for mesh in meshes:
        attributes = {
            "POSITION": accessor(mesh.positions.astype(np.float32), "VEC3", 5126, 34962, True),
            "NORMAL": accessor(mesh.normals.astype(np.float32), "VEC3", 5126, 34962),
            "TEXCOORD_0": accessor(mesh.uvs.astype(np.float32), "VEC2", 5126, 34962),
        }
        indices = accessor(mesh.indices.astype(np.uint32), "SCALAR", 5125, 34963)
        gltf["meshes"].append(
            {
                "name": mesh.name,
                "primitives": [
                    {"attributes": attributes, "indices": indices, "material": mesh.material}
                ],
            }
        )
        gltf["nodes"].append({"name": mesh.name, "mesh": len(gltf["meshes"]) - 1})

    for key in ("textures", "images"):
        if not gltf[key]:
            del gltf[key]
    while len(buffer.data) % 4:
        buffer.data.append(0)
    gltf["buffers"] = [{"byteLength": len(buffer.data)}]
    header = json.dumps(gltf, separators=(",", ":")).encode()
    header += b" " * ((4 - len(header) % 4) % 4)
    with open(path, "wb") as handle:
        total = 12 + 8 + len(header) + 8 + len(buffer.data)
        handle.write(struct.pack("<III", 0x46546C67, 2, total))
        handle.write(struct.pack("<II", len(header), 0x4E4F534A))
        handle.write(header)
        handle.write(struct.pack("<II", len(buffer.data), 0x004E4942))
        handle.write(buffer.data)
