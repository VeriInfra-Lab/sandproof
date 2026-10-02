"""Materialize complete model directories selected by the original task filter."""
import json
from pathlib import Path, PurePosixPath
import subprocess


def select_model_paths(paths, required_tasks):
    required = {task.lower() for task in required_tasks}
    if not required or any(not task for task in required):
        raise ValueError('nonempty benchmark task names required')
    models = {}
    for path in paths:
        parts = PurePosixPath(path).parts
        if len(parts) < 3 or parts[0] != 'results':
            continue
        found = models.setdefault(parts[1], set())
        found.update(task for task in required if task in parts[-1].lower())
    selected = {model for model, found in models.items() if required <= found}
    if not selected:
        raise ValueError('no models contain all benchmark task files')
    # Preserve every revision, metadata file and task file of every candidate.
    # Selection uses filenames only, exactly as the original oracle; no scores.
    return [path for path in paths if len(PurePosixPath(path).parts) >= 3
            and PurePosixPath(path).parts[0] == 'results'
            and PurePosixPath(path).parts[1] in selected], selected


def restore(paths):
    if paths:
        subprocess.run(['git', '--literal-pathspecs', 'restore', '--source=HEAD', '--worktree',
                        '--pathspec-from-file=-', '--pathspec-file-nul'],
                       input=b''.join(path.encode('utf-8') + b'\0' for path in paths), check=True)


def materialize_results(required_tasks):
    tree = subprocess.check_output(['git', 'ls-tree', '-rz', '--name-only', 'HEAD'])
    paths = [path.decode('utf-8') for path in tree.split(b'\0') if path]
    selected, models = select_model_paths(paths, required_tasks)
    # MTEB filters on the name inside model_meta.json, which can differ from
    # the directory name. Preserve all metadata and every matching alias too.
    metadata = [p for p in paths if len(PurePosixPath(p).parts) == 4
                and PurePosixPath(p).parts[0] == 'results'
                and PurePosixPath(p).name == 'model_meta.json']
    restore(metadata)
    names = {model.replace('__', '/') for model in models}
    materialized = set(models)
    for path in metadata:
        name = json.loads(Path(path).read_text())['name'].replace('__', '/')
        if name in names:
            materialized.add(PurePosixPath(path).parts[1])
    selected = [p for p in paths if len(PurePosixPath(p).parts) >= 3
                and PurePosixPath(p).parts[0] == 'results'
                and PurePosixPath(p).parts[1] in materialized]
    restore(selected)
    print(json.dumps({'mteb_results_materialization': {
        'commit': subprocess.check_output(['git', 'rev-parse', 'HEAD'], text=True).strip(),
        'required_tasks': required_tasks, 'candidate_models': len(models),
        'materialized_model_directories': len(materialized),
        'preserved_model_metadata_files': len(metadata),
        'selected_files': len(selected), 'complete_git_objects': True,
        'partial_worktree': True}}), flush=True)


if __name__ == '__main__':
    import mteb
    materialize_results([task.metadata.name
                         for task in mteb.get_benchmark('MTEB(Scandinavian, v1)').tasks])
