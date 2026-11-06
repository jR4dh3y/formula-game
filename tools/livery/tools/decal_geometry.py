import numpy as np


def triangles_intersect_projected_rect(triangles, center, base, up, width, height, margin):
    """Broadphase triangles by projected vertex bounds; texel projection clips exact coverage."""
    along = triangles @ base - center.dot(base)
    across = triangles @ up - center.dot(up)
    return (
        (along.min(axis=1) <= width / 2 + margin)
        & (along.max(axis=1) >= -width / 2 - margin)
        & (across.min(axis=1) <= height / 2 + margin)
        & (across.max(axis=1) >= -height / 2 - margin)
    )
