use std::fs;
use std::path::Path;

#[test]
fn reusable_workflow_rejects_fork_prs_and_runs_full_chain() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let workflow = fs::read_to_string(root.join(".github/workflows/slopgate.yml")).unwrap();
    assert!(workflow.contains("workflow_call:"));
    assert!(workflow.contains("github.event.pull_request.head.repo.full_name == github.repository"));
    assert!(workflow.contains("persist-credentials: false"));
    let self_test = workflow.find("slopgate --self-test").unwrap();
    let harvest = workflow.find("slopgate harvest --check").unwrap();
    let scan = workflow
        .find("slopgate scan --scope repo --tier commit --format github")
        .unwrap();
    assert!(self_test < harvest && harvest < scan);
}

#[test]
fn release_smokes_staged_native_binary_before_upload() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let workflow = fs::read_to_string(root.join(".github/workflows/release.yml")).unwrap();
    let stage = workflow
        .find("node scripts/build-npm-packages.mjs --stage")
        .unwrap();
    let smoke = workflow
        .find("node scripts/build-npm-packages.mjs --smoke")
        .unwrap();
    let upload = workflow.find("actions/upload-artifact@v7").unwrap();
    assert!(stage < smoke && smoke < upload);
}
