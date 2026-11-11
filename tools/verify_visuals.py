#!/usr/bin/env python3
"""Run quiet Vulkan comparisons inside a fixed-size Weston session (no builds)."""
import argparse
import json
import os
from pathlib import Path
import subprocess
import time


def competitors(excluded=()):
    found = []
    for entry in Path('/proc').iterdir():
        if not entry.name.isdigit() or int(entry.name) in excluded:
            continue
        try:
            executable = Path(os.fsdecode((entry / 'cmdline').read_bytes().split(b'\0')[0])).name
            if executable in ('cargo', 'rustc', 'cargo-clippy', 'clippy-driver', 'f1-the-game') or executable.startswith(('f1_the_game-', 'racecraft_sim-')):
                found.append((int(entry.name), executable))
        except (OSError, IndexError):
            pass
    return found


def video(preset, enabled):
    value = dict(anti_alias='Taa', sharpening=0, shadow_resolution=2048, shadow_cascades=2,
                 shadow_distance=70, shadow_filter='Soft', contact_shadows=True,
                 ambient_occlusion='Off', sky='Medium', texture='Half', ai_detail='Medium',
                 tonemapping='AcesFitted', exposure=0, saturation=1.05, contrast=1.01,
                 temperature=0, bloom=.08, vignette=0, chromatic_aberration=0, motion_blur=.18,
                 depth_of_field=False, bokeh=False, focus_distance=30, haze=.018, effects='High',
                 surface_detail=True, cinematic_lighting=True, camera_motion=True)
    if preset == 'low':
        value.update(anti_alias='Fxaa', shadow_resolution=1024, shadow_cascades=2,
                     shadow_distance=90, shadow_filter='Hard', contact_shadows=False,
                     texture='Quarter', ai_detail='Low', saturation=1, contrast=1,
                     haze=0, effects='Off', surface_detail=False, cinematic_lighting=False,
                     camera_motion=False, bloom=0, motion_blur=0)
    if preset == 'ultra':
        value.update(shadow_resolution=4096, shadow_cascades=4, shadow_distance=90,
                     ambient_occlusion='High', sky='High', ai_detail='High')
    if not enabled:
        value.update(saturation=1, contrast=1, bloom=0, haze=0, motion_blur=0,
                     effects='Off', surface_detail=False, cinematic_lighting=False, camera_motion=False)
    return value


def run(args, name, command, settings, extra=None):
    folder = args.output / name
    if (folder / 'validated').exists():
        return
    while competitors():
        print('Waiting for', competitors(), flush=True)
        time.sleep(5)
    subprocess.run(['pgrep', '-fa', 'cargo|rustc|f1-the-game|f1_the_game'], stdout=(args.output / f'{name}-processes.txt').open('w'), check=False)
    subprocess.run(['nvidia-smi'], stdout=(args.output / f'{name}-gpu.txt').open('w'), check=True)
    (args.output / f'{name}-video.json').write_text(json.dumps(settings, indent=2))
    env = os.environ.copy()
    for key in ('WAYLAND_DISPLAY', 'F1_WAYLAND', 'F1_PROFILE', 'F1_VISUAL_TRACE'):
        env.pop(key, None)
    env['F1_VIDEO'] = json.dumps(settings)
    env.update(extra or {})
    contamination = []
    memory = []
    with (args.output / f'{name}.log').open('w') as log:
        child = subprocess.Popen(command, env=env, stdout=log, stderr=subprocess.STDOUT)
        try:
            while child.poll() is None:
                contamination.extend(competitors((child.pid,)))
                reading = subprocess.run(['nvidia-smi', '--query-gpu=memory.used,utilization.gpu', '--format=csv,noheader,nounits'], capture_output=True, text=True, check=True).stdout.strip()
                memory.append(dict(monotonic=time.monotonic(), reading=reading))
                time.sleep(2)
        finally:
            if child.poll() is None:
                child.terminate()
                try:
                    child.wait(timeout=10)
                except subprocess.TimeoutExpired:
                    child.kill()
                    child.wait()
    (args.output / f'{name}-runtime.json').write_text(json.dumps(dict(contamination=contamination, gpu_samples=memory), indent=2))
    if child.returncode:
        raise RuntimeError(f'{name} exited {child.returncode}; read its log')
    if contamination:
        raise RuntimeError(f'{name} was contaminated; preserve it and rerun into a fresh output directory')
    report = json.loads((folder / ('telemetry.json' if args.mode == 'captures' else 'benchmark.json')).read_text())
    if args.mode != 'captures':
        assert report['resolution'] == [1920, 1080]
        assert report['preflight_matches_measured']
        print(name, report['metrics'], flush=True)
    else:
        assert len(report) == 9
        for frame in report:
            assert (folder / f"{frame['name']}.png").is_file()
        print(name, len(report), 'scripted frames', flush=True)
    (folder / 'validated').touch()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('mode', choices=('perf', 'profile', 'captures', 'ultra'))
    parser.add_argument('output', type=Path)
    parser.add_argument('--binary', type=Path, default=Path('target/release/f1-the-game'))
    parser.add_argument('--replay-binary', type=Path)
    args = parser.parse_args()
    args.output.mkdir(parents=True, exist_ok=True)
    args.binary = args.binary.resolve()
    assets = str(Path('assets').resolve())
    if args.mode == 'captures':
        if not args.replay_binary:
            parser.error('--replay-binary is required')
        for preset in ('high', 'low', 'ultra'):
            for enabled in (False, True):
                name = f'{"after" if enabled else "before"}-{preset}'
                run(args, name, [str(args.replay_binary.resolve()), '--exact', 'visual_review::scripted_captures', '--ignored', '--nocapture'], video(preset, enabled), dict(F1_TEST_ASSETS=assets, F1_TEST_OUTPUT=str((args.output / name).resolve()), F1_REVIEW_PRESET=preset, F1_DETERMINISTIC='1'))
    else:
        presets = ('high', 'low') if args.mode == 'perf' else ('ultra',) if args.mode == 'ultra' else ('high',)
        for preset in presets:
            for enabled in ((False, True) if args.mode == 'perf' else (True,)):
                for repeat in ((1, 2) if args.mode == 'perf' else (1,)):
                    name = f'{args.mode}-{preset}-{"on" if enabled else "off"}-{repeat}'
                    extra = dict(F1_PROFILE='1', F1_VISUAL_TRACE=str((args.output / f'{name}-frames.jsonl').resolve())) if args.mode == 'profile' else {}
                    command = [str(args.binary), '--assets', assets, '--camera', 'chase', '--size', '1920x1080', '--graphics', preset, '--mute', '--benchmark', str((args.output / name).resolve()), '--benchmark-seed', '24301']
                    run(args, name, command, video(preset, enabled), extra)


if __name__ == '__main__':
    main()
