#!/usr/bin/env python3
"""Stage only converted simulation inputs for the Render dedicated server."""
import argparse
import hashlib
import json
from pathlib import Path


def prepare(assets: Path, destination: Path, car: str, track: str) -> None:
    # Names become path components in both the server and Docker build context.
    if not track or len(track) > 32 or not all(c.isascii() and (c.isalnum() or c == '_') for c in track):
        raise ValueError('invalid circuit name')
    parts = [f'tracks/{track}/track.json', f'tracks/{track}/collision.bin', f'cars/{car}/car.json']
    roster = 'cars/gen22/teams.json'
    if car == 'gen22' or car == 'gen26' and (assets / roster).is_file():
        parts.append(roster)
    payloads = {part: (assets / part).read_bytes() for part in parts}
    for name, data in payloads.items():
        if name.endswith('.json'):
            json.loads(data)
        elif data[:4] != b'F1CL':
            raise ValueError(f'{name}: not converted collision data')
    # Fail before writing if staging accidentally points into the source tree.
    if destination.resolve().is_relative_to(assets.resolve()) or assets.resolve().is_relative_to(destination.resolve()):
        raise ValueError('source assets and deployment directory must be separate')
    destination.mkdir(parents=True, exist_ok=True)
    manifest = {}
    for part, data in payloads.items():
        target = destination / 'assets' / part
        target.parent.mkdir(parents=True, exist_ok=True)
        target.write_bytes(data)
        manifest[part] = hashlib.sha256(data).hexdigest()
    rules = dict(track=track, car=car, laps=3, ai_cars=0, lock_aids=True,
                 assists=dict(automatic_gears=True, steering=0, braking=0, traction=0, stability=0, drs_anywhere=False),
                 corner_braking=1, setup=dict(front_wing=5, rear_wing=5, compound="Medium"))
    (destination / 'rules.json').write_text(json.dumps(rules, indent=2) + '\n')
    (destination / 'asset-sha256.json').write_text(json.dumps(manifest, indent=2) + '\n')
    print(f'Staged {car}/{track} server inputs in {destination}')
    print('Clients need the same converted simulation assets and a build from this source revision.')


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--assets', required=True, type=Path)
    parser.add_argument('--car', choices=['rb6', 'gen22', 'gen26'], default='rb6')
    parser.add_argument('--track', default='spa')
    args = parser.parse_args()
    root = Path(__file__).resolve().parents[2]
    prepare(args.assets, root / 'deploy/render', args.car, args.track)


if __name__ == '__main__':
    main()
