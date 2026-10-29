"""Minimal AC3D (.ac) reader for Speed Dreams track exports.

Produces flat, world-space triangle soups grouped per object, which the track
converter then classifies into visual materials and physics surfaces.
"""

from __future__ import annotations

from dataclasses import dataclass, field

import numpy as np


@dataclass
class AcObject:
    name: str
    texture: str | None
    # (n, 3) world-space positions referenced by `triangles`.
    vertices: np.ndarray
    # (t, 3) vertex indices.
    triangles: np.ndarray
    # (t, 3, 2) per-corner UVs, already in glTF convention (v down).
    uvs: np.ndarray
    # (t,) per-triangle smooth flag and double-sided flag.
    smooth: np.ndarray
    double_sided: np.ndarray
    material: np.ndarray


@dataclass
class AcMaterial:
    rgb: tuple[float, float, float]
    emissive: tuple[float, float, float]
    transparency: float


@dataclass
class AcFile:
    materials: list[AcMaterial] = field(default_factory=list)
    objects: list[AcObject] = field(default_factory=list)


class _Lines:
    def __init__(self, text: str):
        self.lines = text.splitlines()
        self.index = 0

    def next(self) -> str:
        while self.index < len(self.lines):
            line = self.lines[self.index].strip()
            self.index += 1
            if line:
                return line
        raise EOFError

    def peek(self) -> str | None:
        while self.index < len(self.lines):
            line = self.lines[self.index].strip()
            if line:
                return line
            self.index += 1
        return None


def _parse_material(line: str) -> AcMaterial:
    tokens = line.split('"')[2].split()
    values: dict[str, list[float]] = {}
    key = None
    for token in tokens:
        try:
            values.setdefault(key, []).append(float(token))
        except ValueError:
            key = token
    return AcMaterial(
        rgb=tuple(values.get("rgb", [0.8, 0.8, 0.8])[:3]),
        emissive=tuple(values.get("emis", [0.0, 0.0, 0.0])[:3]),
        transparency=values.get("trans", [0.0])[0],
    )


def _parse_object(lines: _Lines, parent_rot: np.ndarray, parent_loc: np.ndarray, out: AcFile):
    name = ""
    texture = None
    texrep = np.array([1.0, 1.0])
    texoff = np.array([0.0, 0.0])
    rot = np.eye(3)
    loc = np.zeros(3)
    vertices = np.zeros((0, 3))
    tris: list[tuple[int, int, int]] = []
    uvs: list[list[tuple[float, float]]] = []
    smooth: list[bool] = []
    double_sided: list[bool] = []
    materials: list[int] = []

    while True:
        line = lines.next()
        head = line.split()[0]
        if head == "name":
            name = line.split('"')[1]
        elif head == "texture":
            texture = line.split('"')[1]
        elif head == "texrep":
            texrep = np.array([float(v) for v in line.split()[1:3]])
        elif head == "texoff":
            texoff = np.array([float(v) for v in line.split()[1:3]])
        elif head == "rot":
            rot = np.array([float(v) for v in line.split()[1:10]]).reshape(3, 3)
        elif head == "loc":
            loc = np.array([float(v) for v in line.split()[1:4]])
        elif head == "data":
            length = int(line.split()[1])
            consumed = 0
            while consumed < length:
                consumed += len(lines.next()) + 1
        elif head == "url" or head == "crease" or head == "hidden" or head == "locked" or head == "subdiv":
            pass
        elif head == "numvert":
            count = int(line.split()[1])
            vertices = np.array(
                [[float(v) for v in lines.next().split()[:3]] for _ in range(count)]
            ).reshape(count, 3)
        elif head == "numsurf":
            for _ in range(int(line.split()[1])):
                flags = int(lines.next().split()[1], 16)
                material = 0
                refs: list[tuple[int, float, float]] = []
                while True:
                    entry = lines.next()
                    if entry.startswith("mat"):
                        material = int(entry.split()[1])
                    elif entry.startswith("refs"):
                        for _ in range(int(entry.split()[1])):
                            parts = lines.next().split()
                            refs.append((int(parts[0]), float(parts[1]), float(parts[2])))
                        break
                if flags & 0x0F != 0 or len(refs) < 3:
                    continue  # lines and degenerate polygons have no area
                for i in range(1, len(refs) - 1):
                    corner = (refs[0], refs[i], refs[i + 1])
                    tris.append(tuple(c[0] for c in corner))
                    uvs.append(
                        [
                            (
                                c[1] * texrep[0] + texoff[0],
                                1.0 - (c[2] * texrep[1] + texoff[1]),
                            )
                            for c in corner
                        ]
                    )
                    smooth.append(bool(flags & 0x10))
                    double_sided.append(bool(flags & 0x20))
                    materials.append(material)
        elif head == "kids":
            world_rot = parent_rot @ rot
            world_loc = parent_rot @ loc + parent_loc
            if tris:
                world_vertices = vertices @ world_rot.T + world_loc
                out.objects.append(
                    AcObject(
                        name=name,
                        texture=texture,
                        vertices=world_vertices,
                        triangles=np.array(tris, dtype=np.int64),
                        uvs=np.array(uvs, dtype=np.float64),
                        smooth=np.array(smooth),
                        double_sided=np.array(double_sided),
                        material=np.array(materials, dtype=np.int64),
                    )
                )
            for _ in range(int(line.split()[1])):
                kid = lines.next()
                assert kid.startswith("OBJECT"), kid
                _parse_object(lines, world_rot, world_loc, out)
            return
        # Unknown tokens are ignored; AC3D readers are expected to be lenient.


def read_ac(path: str) -> AcFile:
    with open(path, encoding="latin-1") as handle:
        lines = _Lines(handle.read())
    header = lines.next()
    if not header.startswith("AC3D"):
        raise ValueError(f"{path} is not an AC3D file")
    result = AcFile()
    while (line := lines.peek()) is not None:
        lines.next()
        if line.startswith("MATERIAL"):
            result.materials.append(_parse_material(line))
        elif line.startswith("OBJECT"):
            _parse_object(lines, np.eye(3), np.zeros(3), result)
    return result
