"""Shared, Blender-independent UV0 panel geometry and livery format helpers."""
import json
from pathlib import Path

import numpy as np
from PIL import Image


PARTS = ('main_body', 'front_wing', 'rear_wing', 'mirrors', 'floor',
         'cockpit_foam', 'winglets', 'front_covers', 'rear_covers')
REPO = Path(__file__).resolve().parents[2]
DEFAULT_TEMPLATES = REPO / 'raw/gen22/uv-templates'


def load_index(directory):
    index = json.loads((Path(directory) / 'panels.json').read_text())
    if index.get('schema') != 1 or index.get('uv_channel') != 0:
        raise ValueError('expected a schema 1 UV0 panel index')
    return index


def require_native_parts(directory):
    """Reject incomplete preview inputs before Blender creates neutral fallback paint."""
    dimensions = set()
    for part in PARTS:
        path = Path(directory)/f'{part}.png'
        with Image.open(path) as source:
            dimensions.add(source.size)
            source.verify()
    if len(dimensions) != 1:
        raise ValueError('native preview parts must have matching dimensions')
    width, height = dimensions.pop()
    if width != height or width < 1 or width > 4096:
        raise ValueError('native preview parts must be square and at most 4096')


def panel_lookup(index, name, part=None):
    try:
        panel = index['panels'][name]
    except KeyError as error:
        raise ValueError(f'unknown UV panel: {name}') from error
    if part is not None and panel['part'] != part:
        raise ValueError(f'{name} belongs to {panel["part"]}, not {part}')
    return panel


def panel_points(panel, points, flip='auto'):
    """Panel coordinates read upright in the physical view, including UV reflection.

    frame_uv columns are the full panel right/down vectors. Their signed
    determinant already includes the source reflection. Never flip twice.
    'none' removes that reflection; 'x' adds an explicit horizontal reflection.
    """
    points = np.asarray(points, dtype=float).copy()
    if flip not in ('auto', 'none', 'x'):
        raise ValueError(f'unknown flip: {flip}')
    if flip == 'x' or (flip == 'none' and panel['mirrored']):
        points[..., 0] = 1 - points[..., 0]
    frame = np.asarray(panel['frame_uv'])
    return points @ frame[:, 1:].T + frame[:, 0]


def sha256(path):
    import hashlib
    digest = hashlib.sha256()
    with Path(path).open('rb') as stream:
        for block in iter(lambda: stream.read(1024*1024), b''):
            digest.update(block)
    return digest.hexdigest()
