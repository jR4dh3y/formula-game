"""Bake the driver's steering and shift poses as morph targets (imported by prepare_gen22.py).

The source rig pins both IK hands to the `steering_wheel` bone, so turning that
bone moves the arms with the rim. Each pose is stored as per-vertex offsets in
point attributes, which survive decimation, joins and bmesh clean-up, and is
turned into shape keys only around each glTF export.

Two more targets curl the index and middle fingers that pull the right
(upshift) and left (downshift) paddles, baked with the wheel straight.

Angles are steering-wheel rotations about the exported `steering_wheel` node's
+Z axis (glTF space), the same quantity the game computes as -6 * steer.
"""

import numpy as np
import bpy

# Half lock and full lock both ways; the game's 6:1 ratio reaches about 2.16 rad.
ANGLES = (-2.16, -1.08, 1.08, 2.16)
HEAD_YAW = 0.26
SHIFTS = ('up', 'down')
# Negative X on a rigify finger master curls the finger towards the palm.
FINGER_PULL = -0.45


def _attribute(index):
    return f'steer_{index}'


def _world_positions(source, depsgraph):
    mesh = bpy.data.meshes.new_from_object(
        source.evaluated_get(depsgraph), preserve_all_data_layers=True, depsgraph=depsgraph)
    co = np.empty(len(mesh.vertices) * 3)
    mesh.vertices.foreach_get('co', co)
    bpy.data.meshes.remove(mesh)
    co = co.reshape(-1, 3)
    matrix = np.array(source.matrix_world)
    return co @ matrix[:3, :3].T + matrix[:3, 3]


def capture(sources):
    """Return {source name: [world-space vertex offsets per angle]}.

    Leaves the rig in its straight-ahead pose with the steering mapping muted,
    which is exactly the pose the converter evaluates afterwards.
    """
    wheel = bpy.data.objects['steering_and_drs'].pose.bones['steering_wheel']
    rig = bpy.data.objects['rig']
    head = rig.pose.bones['head']
    # The driver's right hand (.R) works the upshift paddle.
    fingers = {shift: [rig.pose.bones[f'{finger}.01_master.{side}'] for finger in ('f_index', 'f_middle')]
               for shift, side in zip(SHIFTS, 'RL')}
    for bone in (wheel, head, *fingers['up'], *fingers['down']):
        for constraint in bone.constraints:
            if constraint.type == 'TRANSFORM':
                constraint.mute = True
        bone.rotation_mode = 'XYZ'

    def pose(angle, shift=None):
        # Bone +Y runs from the hub back towards the driver: glTF +Z reversed.
        wheel.rotation_euler = (0, -angle, 0)
        head.rotation_euler = (0, 0, -HEAD_YAW * max(-1., min(1., angle / ANGLES[2])))
        for key, bones in fingers.items():
            for bone in bones:
                bone.rotation_euler = (FINGER_PULL if key == shift else 0., 0, 0)
        bpy.context.view_layer.update()
        depsgraph = bpy.context.evaluated_depsgraph_get()
        return {source.name: _world_positions(source, depsgraph) for source in sources}

    rest = pose(0.)
    offsets = {name: [] for name in rest}
    for angle, shift in [(angle, None) for angle in ANGLES] + [(0., shift) for shift in SHIFTS]:
        posed = pose(angle, shift)
        for name, positions in posed.items():
            if positions.shape != rest[name].shape:
                raise RuntimeError(f'{name}: posing changed the vertex count')
            offsets[name].append(positions - rest[name])
    pose(0.)
    return offsets


def attach(obj, offsets, rotation):
    """Store offsets on a converted mesh; `rotation` is the converter's 3x3 correction."""
    rotation = np.array(rotation)
    for index, offset in enumerate(offsets):
        if len(offset) != len(obj.data.vertices):
            raise RuntimeError(f'{obj.name}: converted mesh no longer matches the rig')
        attribute = obj.data.attributes.new(_attribute(index), 'FLOAT_VECTOR', 'POINT')
        attribute.data.foreach_set('vector', (offset @ rotation.T).ravel())


def metadata():
    """car.json keys: targets are the steering poses, then the shift poses."""
    return {'driver_poses': list(ANGLES), 'driver_shift_poses': list(SHIFTS)}


def to_shape_keys(obj):
    """Turn the stored offsets into shape keys for export. No-op without offsets."""
    labels = [f'steer_{angle:+.2f}' for angle in ANGLES] + [f'shift_{shift}' for shift in SHIFTS]
    names = [_attribute(i) for i in range(len(labels))]
    if not all(name in obj.data.attributes for name in names):
        return False
    count = len(obj.data.vertices)
    base = np.empty(count * 3)
    obj.data.vertices.foreach_get('co', base)
    obj.shape_key_add(name='Basis', from_mix=False)
    for name, label in zip(names, labels):
        offset = np.empty(count * 3)
        obj.data.attributes[name].data.foreach_get('vector', offset)
        key = obj.shape_key_add(name=label, from_mix=False)
        key.data.foreach_set('co', base + offset)
    return True


def clear_shape_keys(obj):
    if obj.data.shape_keys:
        obj.shape_key_clear()


# The source models both shift paddles as one bar behind the wheel; the cut at
# the hub is hidden by the wheel body. Pulled tips travel about 1 cm.
PADDLE_SOURCE = 'paddle_gear_shifter.002'
PADDLE_PULL = 0.1
PADDLE_HINGE_OFFSET = 0.035


def split_paddles(obj, hub_x):
    """Cut a converted paddle bar at the hub into `paddle_up` (driver's right, -X)
    and `paddle_down`. Returns ({node: object}, {node: hinge}, car.json keys).

    Each hinge sits just inboard of the wheel body edge, so the outer tip swings
    back towards the driver about a vertical axis (glTF +Y).
    """
    import bmesh
    from mathutils import Vector
    halves = {}
    for node, keep_left in (('paddle_up', False), ('paddle_down', True)):
        mesh = obj.data.copy()
        bm = bmesh.new()
        bm.from_mesh(mesh)
        geom = list(bm.verts) + list(bm.edges) + list(bm.faces)
        bmesh.ops.bisect_plane(bm, geom=geom, dist=1e-6, plane_co=(hub_x, 0, 0), plane_no=(1, 0, 0),
                               clear_inner=keep_left, clear_outer=not keep_left)
        bm.to_mesh(mesh)
        bm.free()
        half = bpy.data.objects.new(node, mesh)
        for collection in obj.users_collection:
            collection.objects.link(half)
        halves[node] = half
    bpy.data.objects.remove(obj, do_unlink=True)
    hinges = {}
    for node, half in halves.items():
        points = [v.co for v in half.data.vertices]
        if not points:
            raise RuntimeError(f'{node}: paddle cut left no geometry')
        side = 1 if node == 'paddle_down' else -1
        hinges[node] = Vector((hub_x + side * PADDLE_HINGE_OFFSET,
                               sum(p.y for p in points) / len(points),
                               sum(p.z for p in points) / len(points)))
    # Blender Z maps to glTF +Y with the same handedness; the right (-X) tip
    # moves towards the driver (+Y in Blender) for a negative turn.
    keys = {'paddles': {'up_angle': -PADDLE_PULL, 'down_angle': PADDLE_PULL}}
    return halves, hinges, keys
