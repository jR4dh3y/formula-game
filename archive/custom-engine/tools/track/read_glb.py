"""Memory-mapped, static glTF reader. Never executes source asset code."""

from __future__ import annotations

import io
import json
import mmap
import struct
from pathlib import Path

import numpy as np
from PIL import Image


class Glb:
    def __init__(self, path):
        with Path(path).open("rb") as source:
            self.data = mmap.mmap(source.fileno(), 0, access=mmap.ACCESS_READ)
        magic, version, size = struct.unpack_from("<III", self.data)
        if magic != 0x46546C67 or version != 2 or size != len(self.data):
            raise ValueError("invalid GLB header")
        length, kind = struct.unpack_from("<II", self.data, 12)
        if kind != 0x4E4F534A:
            raise ValueError("missing JSON chunk")
        self.json = json.loads(self.data[20 : 20 + length])
        self.base = 28 + length
        if self.json.get("extensionsRequired"):
            raise ValueError("compressed/required GLB extensions are unsupported")

    def accessor(self, index):
        a = self.json["accessors"][index]
        if "sparse" in a:
            raise ValueError("sparse accessor unsupported")
        v = self.json["bufferViews"][a["bufferView"]]
        dtype = np.dtype(
            {
                5120: "i1",
                5121: "u1",
                5122: "<i2",
                5123: "<u2",
                5125: "<u4",
                5126: "<f4",
            }[a["componentType"]]
        )
        components = {"SCALAR": 1, "VEC2": 2, "VEC3": 3, "VEC4": 4}[a["type"]]
        offset = self.base + v.get("byteOffset", 0) + a.get("byteOffset", 0)
        stride = v.get("byteStride", dtype.itemsize * components)
        array = np.ndarray(
            (a["count"], components), dtype, self.data, offset, (stride, dtype.itemsize)
        )
        if a.get("normalized") and dtype.kind != "f":
            array = array.astype(np.float32) / np.iinfo(dtype).max
        return array

    def image(self, material, cap=1024):
        pbr = material.get("pbrMetallicRoughness", {})
        tex = pbr.get("baseColorTexture", {}).get("index")
        if tex is None:
            return None
        image = self.json["images"][self.json["textures"][tex]["source"]]
        if "uri" in image:
            raise ValueError("external image URI unsupported; embed source textures")
        view = self.json["bufferViews"][image["bufferView"]]
        start = self.base + view.get("byteOffset", 0)
        with Image.open(
            io.BytesIO(self.data[start : start + view["byteLength"]])
        ) as source:
            source.thumbnail((cap, cap), Image.Resampling.LANCZOS)
            return source.convert(
                "RGBA"
                if "A" in source.getbands() or "transparency" in source.info
                else "RGB"
            )

    def primitives(self, scale=1.0):
        def visit(index, parent):
            node = self.json["nodes"][index]
            if "matrix" in node:
                local = np.array(node["matrix"]).reshape(4, 4).T
            else:
                x, y, z, w = node.get("rotation", [0, 0, 0, 1])
                local = np.eye(4)
                local[:3, :3] = np.array(
                    [
                        [
                            1 - 2 * (y * y + z * z),
                            2 * (x * y - z * w),
                            2 * (x * z + y * w),
                        ],
                        [
                            2 * (x * y + z * w),
                            1 - 2 * (x * x + z * z),
                            2 * (y * z - x * w),
                        ],
                        [
                            2 * (x * z - y * w),
                            2 * (y * z + x * w),
                            1 - 2 * (x * x + y * y),
                        ],
                    ]
                ) @ np.diag(node.get("scale", [1, 1, 1]))
                local[:3, 3] = node.get("translation", [0, 0, 0])
            matrix = parent @ local
            if "mesh" in node:
                for p in self.json["meshes"][node["mesh"]]["primitives"]:
                    if p.get("mode", 4) != 4:
                        continue
                    a = p["attributes"]
                    positions = (
                        self.accessor(a["POSITION"]) @ matrix[:3, :3].T + matrix[:3, 3]
                    ) * scale
                    if "NORMAL" in a:
                        normals = self.accessor(a["NORMAL"]) @ np.linalg.inv(
                            matrix[:3, :3]
                        )
                        normals /= np.maximum(
                            np.linalg.norm(normals, axis=1, keepdims=True), 1e-12
                        )
                    else:
                        normals = np.zeros_like(positions)
                    uvs = (
                        self.accessor(a["TEXCOORD_0"])
                        if "TEXCOORD_0" in a
                        else np.zeros((len(positions), 2))
                    )
                    faces = (
                        self.accessor(p["indices"]).reshape(-1, 3)
                        if "indices" in p
                        else np.arange(len(positions)).reshape(-1, 3)
                    )
                    if np.linalg.det(matrix[:3, :3]) < 0:
                        faces = faces[:, ::-1]
                    yield (
                        p.get("material", 0),
                        positions.astype("f4"),
                        normals.astype("f4"),
                        uvs.astype("f4"),
                        faces,
                    )
            for child in node.get("children", []):
                yield from visit(child, matrix)

        for root in self.json["scenes"][self.json.get("scene", 0)]["nodes"]:
            yield from visit(root, np.eye(4))
