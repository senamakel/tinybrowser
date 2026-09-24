//! Unit tests for question construction, decoding, and deterministic gates.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::collections::{BTreeMap, VecDeque};
use std::sync::Mutex;
use std::time::Duration;

use serde_json::json;
use tinybrowser::{Action, ActionOutcome, PageState, ScrollDirection, SessionId, Snapshot, Target};

#[test]
fn direct_engine_errors_still_convert_to_controller_errors() {
    let error: Error = tinybrowser::Error::not_actionable("stale target").into();
    assert!(matches!(error, Error::Browser { .. }));
}
use tinyjevclient::{
    Answer, ChoiceAnswer, EvaluationResponse, EvaluationResult, NoulAnswer, Question, Usage,
};

use super::*;
use crate::policy;

fn snapshot() -> Snapshot {
    Snapshot {
        url: "https://example.com/search".to_owned(),
        title: "Search".to_owned(),
        sequence: 4,
        tree: "textbox \"Query\" @e1\nbutton \"Search\" @e2\ncheckbox \"Images\" @e3".to_owned(),
        refs: vec![
            element("e1", "textbox", "Query"),
            element("e2", "button", "Search"),
            element("e3", "checkbox", "Images"),
            element("e4", "heading", "Results"),
            element("e5", "link", "Advanced search"),
            element("e6", "searchbox", "Site"),
            element("e7", "switch", "Safe search"),
        ],
        truncated: false,
    }
}

fn single_target_snapshot() -> Snapshot {
    let mut value = snapshot();
    value.refs.retain(|element| element.id != "e6");
    value
}

fn element(id: &str, role: &str, name: &str) -> tinybrowser::ElementRef {
    tinybrowser::ElementRef {
        id: id.to_owned(),
        role: role.to_owned(),
        name: name.to_owned(),
    }
}

fn result(answers: BTreeMap<String, Answer>) -> EvaluationResult {
    EvaluationResult {
        response: EvaluationResponse {
            model: "jev-latest".to_owned(),
            answers,
            usage: Usage {
                input_tokens: Some(42),
                output_tokens: Some(7),
            },
        },
        request_id: Some("request-1".to_owned()),
        attempts: 1,
        latency: Duration::from_millis(420),
    }
}

fn choice(selected: &str, options: &[&str]) -> Answer {
    Answer::Choice(ChoiceAnswer {
        choice: selected.to_owned(),
        probabilities: options
            .iter()
            .map(|option| ((*option).to_owned(), f64::from(*option == selected)))
            .collect(),
        confidence: 0.9,
    })
}

fn decision(operation: Operation, target: Option<tinybrowser::ElementRef>) -> Decision {
    Decision {
        operation,
        confidence: 0.9,
        goal_done: 0.1,
        target,
        target_confidence: Some(0.8),
        input_name: None,
        attempts: 1,
        latency: Duration::from_millis(10),
        usage: Usage::default(),
        model: "jev-latest".to_owned(),
        request_id: Some("request-1".to_owned()),
    }
}

fn controller() -> JevController {
    JevController::new(
        tinyjevclient::Client::new(tinyjevclient::ClientConfig::new("test-key"))
            .expect("valid local configuration"),
    )
}

#[derive(Debug)]
struct FakeBrowser {
    snapshots: Mutex<VecDeque<Snapshot>>,
    outcomes: Mutex<VecDeque<tinybrowser::Result<ActionOutcome>>>,
    actions: Mutex<Vec<Action>>,
}

impl FakeBrowser {
    fn new(snapshots: impl IntoIterator<Item = Snapshot>) -> Self {
        Self {
            snapshots: Mutex::new(snapshots.into_iter().collect()),
            outcomes: Mutex::new(VecDeque::new()),
            actions: Mutex::new(Vec::new()),
        }
    }

    fn with_outcomes(
        snapshots: impl IntoIterator<Item = Snapshot>,
        outcomes: impl IntoIterator<Item = tinybrowser::Result<ActionOutcome>>,
    ) -> Self {
        Self {
            snapshots: Mutex::new(snapshots.into_iter().collect()),
            outcomes: Mutex::new(outcomes.into_iter().collect()),
            actions: Mutex::new(Vec::new()),
        }
    }
}

impl BrowserControl for FakeBrowser {
    fn snapshot(
        &self,
        _session: &SessionId,
        _request: &tinybrowser::SnapshotRequest,
    ) -> impl std::future::Future<Output = std::result::Result<Snapshot, BrowserControlError>> {
        std::future::ready(Ok(self
            .snapshots
            .lock()
            .expect("snapshot lock")
            .pop_front()
            .expect("a snapshot for every observation")))
    }

    fn perform(
        &self,
        _session: &SessionId,
        action: &Action,
    ) -> impl std::future::Future<Output = std::result::Result<ActionOutcome, BrowserControlError>>
    {
        self.actions
            .lock()
            .expect("action lock")
            .push(action.clone());
        std::future::ready(
            self.outcomes
                .lock()
                .expect("outcome lock")
                .pop_front()
                .unwrap_or_else(|| {
                    Ok(ActionOutcome::acted(PageState::new(
                        "https://example.com/search",
                    )))
                })
                .map_err(Into::into),
        )
    }
}

#[derive(Debug)]
struct FakeDecisions {
    decisions: Mutex<VecDeque<Result<Decision>>>,
    offered: Mutex<Vec<(bool, bool)>>,
}

impl FakeDecisions {
    fn new(decisions: impl IntoIterator<Item = Decision>) -> Self {
        Self {
            decisions: Mutex::new(decisions.into_iter().map(Ok).collect()),
            offered: Mutex::new(Vec::new()),
        }
    }
}

impl DecisionSource for FakeDecisions {
    fn decide(
        &self,
        _task: &TaskRequest,
        _snapshot: &Snapshot,
        _history: &[StepRecord],
        done_unconfirmed: bool,
        fill_already_entered: bool,
    ) -> impl std::future::Future<Output = Result<Decision>> {
        self.offered
            .lock()
            .expect("offered lock")
            .push((done_unconfirmed, fill_already_entered));
        std::future::ready(
            self.decisions
                .lock()
                .expect("decision lock")
                .pop_front()
                .expect("a decision for every step"),
        )
    }
}

#[test]
fn one_request_contains_operation_targets_inputs_and_completion() {
    let task = TaskRequest::new("search for rust").with_inputs(BTreeMap::from([
        ("query".to_owned(), "local-only-value".to_owned()),
        ("site".to_owned(), "example.com".to_owned()),
    ]));
    let request =
        policy::build_request(&task, &snapshot(), &[], false, false).expect("valid request");

    assert_eq!(request.model, "jev-latest");
    request.validate().expect("meets the client contract");
    assert_eq!(
        request.questions.keys().cloned().collect::<Vec<_>>(),
        vec![
            "check_target",
            "click_target",
            "fill_input",
            "fill_target",
            "goal_done",
            "operation",
        ]
    );
    let Question::Choice(click) = &request.questions["click_target"] else {
        panic!("click_target must be a choice");
    };
    assert_eq!(click.criteria.keys().collect::<Vec<_>>(), vec!["e2", "e5"]);
    let Question::Choice(operation) = &request.questions["operation"] else {
        panic!("operation must be a choice");
    };
    assert!(operation.criteria.contains_key("FILL"));
    assert!(operation.criteria.contains_key("DONE"));
    assert!(
        !serde_json::to_string(&request)
            .expect("serializes")
            .contains("local-only-value")
    );
}

#[test]
fn single_targets_and_inputs_are_resolved_without_invalid_choice_questions() {
    let task = TaskRequest::new("search").with_inputs(BTreeMap::from([(
        "query".to_owned(),
        "local value".to_owned(),
    )]));
    let request = policy::build_request(&task, &single_target_snapshot(), &[], false, false)
        .expect("valid request");

    request.validate().expect("meets the client contract");
    assert!(!request.questions.contains_key("fill_target"));
    assert!(!request.questions.contains_key("fill_input"));
}

#[test]
fn fill_is_not_offered_without_a_caller_supplied_value() {
    let request =
        policy::build_request(&TaskRequest::new("inspect"), &snapshot(), &[], false, false)
            .expect("valid request");
    let Question::Choice(operation) = &request.questions["operation"] else {
        panic!("operation must be a choice");
    };
    assert!(!operation.criteria.contains_key("FILL"));
    assert!(!request.questions.contains_key("fill_input"));
}

#[test]
fn a_fill_decision_maps_names_back_to_a_current_ref_and_local_value() {
    let answers = BTreeMap::from([
        ("operation".to_owned(), choice("FILL", &["FILL", "DONE"])),
        ("fill_target".to_owned(), choice("e1", &["e1"])),
        ("fill_input".to_owned(), choice("query", &["query"])),
        (
            "goal_done".to_owned(),
            Answer::Noul(NoulAnswer { noul: 0.05 }),
        ),
    ]);
    let evaluation = result(answers);
    let task = TaskRequest::new("search").with_inputs(BTreeMap::from([(
        "query".to_owned(),
        "secret local value".to_owned(),
    )]));
    let decoded =
        policy::decode(&evaluation, &single_target_snapshot(), &task).expect("valid decision");

    assert_eq!(decoded.operation, Operation::Fill);
    assert_eq!(
        decoded.target.as_ref().map(|target| target.id.as_str()),
        Some("e1")
    );
    assert_eq!(decoded.input_name.as_deref(), Some("query"));
    assert_eq!(
        decoded.to_action(&task, 250).expect("maps to action"),
        Some(Action::Fill {
            target: Target::reference("e1"),
            value: "secret local value".to_owned(),
        })
    );
}

#[test]
fn an_answer_cannot_name_a_ref_outside_the_current_snapshot() {
    let answers = BTreeMap::from([
        ("operation".to_owned(), choice("CLICK", &["CLICK"])),
        ("click_target".to_owned(), choice("e99", &["e99"])),
        (
            "goal_done".to_owned(),
            Answer::Noul(NoulAnswer { noul: 0.0 }),
        ),
    ]);
    let evaluation = result(answers);
    let task = TaskRequest::new("click");
    let error = policy::decode(&evaluation, &snapshot(), &task).expect_err("unknown ref rejected");
    assert!(
        error
            .to_string()
            .contains("not a compatible current target")
    );
}

#[test]
fn done_requires_the_independent_completion_threshold() {
    let mut selected = decision(Operation::Done, None);
    selected.goal_done = 0.49;
    assert_eq!(
        policy::terminal_status(&selected, 0.5),
        Some(TaskStatus::DoneUnconfirmed)
    );
    selected.goal_done = 0.5;
    assert_eq!(
        policy::terminal_status(&selected, 0.5),
        Some(TaskStatus::Done)
    );
}

#[test]
fn an_unconfirmed_done_retry_offers_actions_but_not_done() {
    let request = policy::build_request(
        &TaskRequest::new("submit the search"),
        &snapshot(),
        &[],
        true,
        true,
    )
    .expect("valid retry request");
    let Question::Choice(operation) = &request.questions["operation"] else {
        panic!("operation must be a choice");
    };
    assert!(operation.criteria.contains_key("CLICK"));
    assert!(!operation.criteria.contains_key("DONE"));
    assert!(!operation.criteria.contains_key("FILL"));
    assert_eq!(request.state["done_unconfirmed"], json!(true));
    request.validate().expect("retry request is valid");
}

#[test]
fn a_completed_single_input_is_not_offered_for_another_fill() {
    let task = TaskRequest::new("submit search")
        .with_inputs(BTreeMap::from([("query".to_owned(), "rust".to_owned())]));
    let request = policy::build_request(&task, &snapshot(), &[], false, true)
        .expect("valid request after fill");
    let Question::Choice(operation) = &request.questions["operation"] else {
        panic!("operation must be a choice");
    };
    assert!(!operation.criteria.contains_key("FILL"));
    assert!(operation.criteria.contains_key("CLICK"));
    assert!(operation.criteria.contains_key("DONE"));
    assert!(!request.questions.contains_key("fill_target"));
    assert_eq!(request.state["fill_already_entered"], json!(true));
    request.validate().expect("meets the client contract");

    let next_page = policy::build_request(&task, &snapshot(), &[], false, false)
        .expect("valid request on another page");
    let Question::Choice(operation) = &next_page.questions["operation"] else {
        panic!("operation must be a choice");
    };
    assert!(operation.criteria.contains_key("FILL"));
}

#[tokio::test]
async fn an_unconfirmed_done_reconsiders_the_visible_submit_button() {
    let before = snapshot();
    let mut filled = before.clone();
    filled
        .tree
        .push_str("\ntextbox \"Query\" value=\"rust\" @e1");
    let mut submitted = filled.clone();
    // A same-URL re-render after the click makes FILL available again.
    submitted.url = filled.url.clone();
    submitted.tree = "heading \"Results\" @e4".to_owned();

    let mut fill = decision(Operation::Fill, Some(element("e1", "textbox", "Query")));
    fill.input_name = Some("query".to_owned());
    let mut unconfirmed = decision(Operation::Done, None);
    unconfirmed.goal_done = 0.1;
    let click = decision(Operation::Click, Some(element("e2", "button", "Search")));
    let mut confirmed = decision(Operation::Done, None);
    confirmed.goal_done = 0.9;
    let browser = FakeBrowser::new([before, filled, submitted]);
    let task = TaskRequest::new("submit the search and reach results")
        .with_inputs(BTreeMap::from([("query".to_owned(), "rust".to_owned())]));

    let decisions = FakeDecisions::new([fill, unconfirmed, click, confirmed]);
    let result = controller()
        .run_with(&browser, &decisions, &SessionId::new("session"), &task)
        .await
        .expect("task result");

    assert_eq!(result.status, TaskStatus::Done);
    assert_eq!(result.steps.len(), 2);
    assert_eq!(result.steps[0].decision.operation, Operation::Fill);
    assert_eq!(result.steps[1].decision.operation, Operation::Click);
    assert_eq!(browser.actions.lock().expect("actions").len(), 2);
    assert_eq!(
        *decisions.offered.lock().expect("offered lock"),
        [(false, false), (false, true), (true, true), (false, false)]
    );
}

#[tokio::test]
async fn repeated_unconfirmed_done_stops_within_the_decision_budget() {
    let mut unconfirmed = decision(Operation::Done, None);
    unconfirmed.goal_done = 0.1;
    let browser = FakeBrowser::new([snapshot()]);
    let result = controller()
        .with_limits(ControlLimits {
            max_steps: 2,
            ..ControlLimits::default()
        })
        .run_with(
            &browser,
            &FakeDecisions::new([unconfirmed.clone(), unconfirmed]),
            &SessionId::new("session"),
            &TaskRequest::new("reach results"),
        )
        .await
        .expect("task result");

    assert_eq!(result.status, TaskStatus::DoneUnconfirmed);
    assert!(result.steps.is_empty());
    assert!(browser.actions.lock().expect("actions").is_empty());
}

#[tokio::test]
async fn filling_one_of_multiple_inputs_keeps_fill_available() {
    let before = snapshot();
    let mut after = before.clone();
    after
        .tree
        .push_str("\ntextbox \"Query\" value=\"rust\" @e1");
    let mut fill = decision(Operation::Fill, Some(element("e1", "textbox", "Query")));
    fill.input_name = Some("query".to_owned());
    let decisions = FakeDecisions::new([fill, decision(Operation::Blocked, None)]);
    let task = TaskRequest::new("fill query and site").with_inputs(BTreeMap::from([
        ("query".to_owned(), "rust".to_owned()),
        ("site".to_owned(), "example.com".to_owned()),
    ]));
    let result = controller()
        .run_with(
            &FakeBrowser::new([before, after]),
            &decisions,
            &SessionId::new("session"),
            &task,
        )
        .await
        .expect("task result");

    assert_eq!(result.status, TaskStatus::Blocked);
    assert_eq!(
        *decisions.offered.lock().expect("offered lock"),
        [(false, false), (false, false)]
    );
}

#[tokio::test]
async fn an_unconfirmed_form_goal_selects_the_current_submit_ref_and_preserves_approval() {
    let before = Snapshot {
        url: "https://www.selenium.dev/selenium/web/web-form.html".to_owned(),
        title: "Web form".to_owned(),
        sequence: 1,
        tree: "textbox \"Text input\" @e4\nbutton \"Reset\" @e22\nbutton \"Submit\" @e33"
            .to_owned(),
        refs: vec![
            element("e4", "textbox", "Text input"),
            element("e22", "button", "Reset"),
            element("e33", "button", "Submit"),
        ],
        truncated: false,
    };
    let mut filled = before.clone();
    filled.sequence = 2;
    filled.tree = "textbox \"Text input\" value=\"OpenHuman browser smoke\" @e4\nbutton \"Reset\" @e22\nbutton \"Submit\" @e33".to_owned();
    let task = TaskRequest::new("Reach the submitted result after entering text").with_inputs(
        BTreeMap::from([("my-text".to_owned(), "OpenHuman browser smoke".to_owned())]),
    );

    let mut fill = decision(
        Operation::Fill,
        Some(element("e4", "textbox", "Text input")),
    );
    fill.input_name = Some("my-text".to_owned());
    let mut unconfirmed = decision(Operation::Done, None);
    unconfirmed.goal_done = 0.1;
    let retry_request = policy::build_request(&task, &filled, &[], true, true)
        .expect("retry uses the current form snapshot");
    let Question::Choice(click_targets) = &retry_request.questions["click_target"] else {
        panic!("click targets must be a choice");
    };
    assert!(click_targets.criteria.contains_key("e33"));
    let submit = policy::decode(
        &result(BTreeMap::from([
            ("operation".to_owned(), choice("CLICK", &["CLICK", "DONE"])),
            ("click_target".to_owned(), choice("e33", &["e22", "e33"])),
            (
                "goal_done".to_owned(),
                Answer::Noul(NoulAnswer { noul: 0.1 }),
            ),
        ])),
        &filled,
        &task,
    )
    .expect("submit ref from current snapshot");
    assert_eq!(
        submit.target.as_ref().map(|target| target.id.as_str()),
        Some("e33")
    );

    let browser = FakeBrowser::new([before.clone(), filled.clone()]);
    let result = controller()
        .run_with(
            &browser,
            &FakeDecisions::new([fill.clone(), unconfirmed.clone(), submit.clone()]),
            &SessionId::new("session"),
            &task,
        )
        .await
        .expect("task result");

    assert_eq!(result.status, TaskStatus::NeedsConfirmation);
    assert_eq!(result.steps.len(), 1);
    assert_eq!(result.steps[0].decision.operation, Operation::Fill);
    assert_eq!(
        result
            .pending
            .as_ref()
            .and_then(|decision| decision.target.as_ref())
            .map(|target| target.id.as_str()),
        Some("e33")
    );
    assert_eq!(browser.actions.lock().expect("actions").len(), 1);

    let mut submitted = filled.clone();
    submitted.url = "https://www.selenium.dev/selenium/web/submitted-form.html".to_owned();
    submitted.tree = "heading \"Form submitted\"\ntext \"Received!\"".to_owned();
    let mut confirmed = decision(Operation::Done, None);
    confirmed.goal_done = 0.9;
    let authorized_browser = FakeBrowser::new([before, filled, submitted]);
    let authorized = controller()
        .run_with(
            &authorized_browser,
            &FakeDecisions::new([fill, unconfirmed, submit, confirmed]),
            &SessionId::new("session"),
            &task.allowing_irreversible(),
        )
        .await
        .expect("authorized task result");
    assert_eq!(authorized.status, TaskStatus::Done);
    assert_eq!(authorized.steps.len(), 2);
    assert_eq!(
        authorized.steps[1]
            .decision
            .target
            .as_ref()
            .map(|target| target.id.as_str()),
        Some("e33")
    );
    assert!(authorized.final_snapshot.tree.contains("Received!"));
    assert_eq!(authorized_browser.actions.lock().expect("actions").len(), 2);
}

#[test]
fn irreversible_policy_covers_consequential_clicks_and_unlabeled_controls() {
    assert!(policy::is_irreversible(&decision(
        Operation::Click,
        Some(element("e1", "button", "Place order"))
    )));
    assert!(policy::is_irreversible(&decision(
        Operation::Click,
        Some(element("e1", "button", "Place order:"))
    )));
    for label in [
        "Submit application",
        "Submit payment",
        "Submit order",
        "Submit transfer",
        "Transfer funds",
        "Authorize payment",
        "Approve access",
        "Confirm transfer",
        "Save changes",
        "Share document",
        "Invite member",
    ] {
        assert!(
            policy::is_irreversible(&decision(
                Operation::Click,
                Some(element("e1", "button", label))
            )),
            "{label} must require confirmation"
        );
    }
    assert!(policy::is_irreversible(&decision(
        Operation::Click,
        Some(element("e1", "button", "  "))
    )));
    assert!(policy::is_irreversible(&decision(
        Operation::Click,
        Some(element("e1", "menuitem", ""))
    )));
    assert!(!policy::is_irreversible(&decision(
        Operation::Click,
        Some(element("e2", "link", "Order history"))
    )));
    assert!(!policy::is_irreversible(&decision(
        Operation::Click,
        Some(element("e2", "button", "Search"))
    )));
    for label in ["Submit search", "Submit query", "Submit filters"] {
        assert!(
            !policy::is_irreversible(&decision(
                Operation::Click,
                Some(element("e2", "button", label))
            )),
            "{label} should remain available"
        );
    }
    assert!(policy::is_irreversible(&decision(
        Operation::Click,
        Some(element("e2", "link", ""))
    )));
    assert!(!policy::is_irreversible(&decision(
        Operation::Fill,
        Some(element("e3", "textbox", "Send to"))
    )));
}

#[test]
fn waiting_and_visible_change_reset_the_stuck_counter() {
    assert_eq!(policy::next_unchanged(1, Operation::Click, false), 2);
    assert_eq!(policy::next_unchanged(2, Operation::Click, true), 0);
    assert_eq!(policy::next_unchanged(2, Operation::Wait, false), 0);
}

#[test]
fn page_change_uses_agent_visible_state_not_snapshot_generation() {
    let before = snapshot();
    let mut after = before.clone();
    after.sequence += 1;
    assert!(!policy::page_changed(&before, &after));
    after.tree.push_str("\nlink \"Result\" @e5");
    assert!(policy::page_changed(&before, &after));
}

#[test]
fn non_target_operations_map_to_typed_browser_actions() {
    let task = TaskRequest::new("move");
    assert_eq!(
        decision(Operation::ScrollDown, None)
            .to_action(&task, 250)
            .expect("maps"),
        Some(Action::Scroll {
            direction: ScrollDirection::Down,
            pixels: None,
            target: None,
        })
    );
    assert_eq!(
        decision(Operation::Done, None)
            .to_action(&task, 250)
            .expect("terminal"),
        None
    );
}

#[test]
fn task_debug_redacts_input_values() {
    let task = TaskRequest::new("log safely").with_inputs(BTreeMap::from([(
        "password".to_owned(),
        "do-not-log".to_owned(),
    )]));
    let rendered = format!("{task:?}");
    assert!(rendered.contains("password"));
    assert!(!rendered.contains("do-not-log"));
}

#[test]
fn request_state_includes_bounded_recent_history() {
    let record = StepRecord {
        step: 1,
        decision: decision(Operation::Click, Some(element("e2", "button", "Search"))),
        page_changed: true,
        outcome: StepOutcome::Acted,
    };
    let history = vec![record; 10];
    let request = policy::build_request(
        &TaskRequest::new("search"),
        &snapshot(),
        &history,
        false,
        false,
    )
    .expect("valid request");
    assert_eq!(
        request.state["recent_actions"].as_array().map(Vec::len),
        Some(8)
    );
    assert_eq!(request.state["page"]["title"], json!("Search"));
}

#[test]
fn every_operation_key_round_trips() {
    for operation in [
        Operation::Click,
        Operation::Fill,
        Operation::Check,
        Operation::ScrollDown,
        Operation::ScrollUp,
        Operation::Back,
        Operation::Wait,
        Operation::Done,
        Operation::Blocked,
    ] {
        assert_eq!(Operation::from_key(operation.key()), Some(operation));
    }
    assert_eq!(Operation::from_key("UNKNOWN"), None);
}

#[test]
fn all_action_operations_map_to_typed_actions() {
    let task = TaskRequest::new("act")
        .with_inputs(BTreeMap::from([("value".to_owned(), "entered".to_owned())]));
    let target = Some(element("e1", "button", "Act"));
    let cases = [
        (Operation::Click, true),
        (Operation::Check, true),
        (Operation::ScrollUp, true),
        (Operation::Back, true),
        (Operation::Wait, true),
        (Operation::Blocked, false),
    ];
    for (operation, has_action) in cases {
        assert_eq!(
            decision(operation, target.clone())
                .to_action(&task, 25)
                .expect("valid mapping")
                .is_some(),
            has_action
        );
    }

    let mut fill = decision(Operation::Fill, Some(element("e1", "textbox", "Value")));
    fill.input_name = Some("value".to_owned());
    assert!(matches!(
        fill.to_action(&task, 25).expect("valid fill"),
        Some(Action::Fill { .. })
    ));
}

#[test]
fn task_builders_and_defaults_are_safe() {
    let task = TaskRequest::new("submit").allowing_irreversible();
    assert!(task.allow_irreversible);
    assert!(task.inputs.is_empty());
    assert_eq!(ControlLimits::default().max_steps, 30);
}

#[tokio::test]
async fn the_runner_accepts_only_independently_confirmed_done() {
    let mut done = decision(Operation::Done, None);
    done.goal_done = 0.8;
    let result = controller()
        .run_with(
            &FakeBrowser::new([snapshot()]),
            &FakeDecisions::new([done]),
            &SessionId::new("session"),
            &TaskRequest::new("finish"),
        )
        .await
        .expect("task result");

    assert_eq!(result.status, TaskStatus::Done);
    assert!(result.steps.is_empty());
}

#[tokio::test]
async fn the_runner_stops_before_an_irreversible_click() {
    for (role, label) in [
        ("button", "Delete account"),
        ("button", "Submit"),
        ("button", "Transfer"),
        ("button", "Authorize"),
        ("button", ""),
        ("link", ""),
    ] {
        let click = decision(Operation::Click, Some(element("e8", role, label)));
        let browser = FakeBrowser::new([snapshot()]);
        let result = controller()
            .run_with(
                &browser,
                &FakeDecisions::new([click.clone()]),
                &SessionId::new("session"),
                &TaskRequest::new("complete the requested action"),
            )
            .await
            .expect("task result");

        assert_eq!(result.status, TaskStatus::NeedsConfirmation, "{label}");
        assert_eq!(result.pending, Some(click), "{label}");
        assert!(result.steps.is_empty(), "{label}");
        assert!(
            browser.actions.lock().expect("action lock").is_empty(),
            "{label}"
        );
    }
}

#[tokio::test]
async fn the_runner_can_submit_a_search_without_confirmation() {
    let browser = FakeBrowser::new([snapshot(), snapshot()]);
    let result = controller()
        .with_limits(ControlLimits {
            max_steps: 1,
            ..ControlLimits::default()
        })
        .run_with(
            &browser,
            &FakeDecisions::new([decision(
                Operation::Click,
                Some(element("e8", "button", "Submit search")),
            )]),
            &SessionId::new("session"),
            &TaskRequest::new("search"),
        )
        .await
        .expect("task result");

    assert_eq!(result.status, TaskStatus::Budget);
    assert_eq!(result.steps.len(), 1);
    assert_eq!(browser.actions.lock().expect("action lock").len(), 1);
}

#[tokio::test]
async fn unchanged_actions_stop_as_stuck() {
    let limits = ControlLimits {
        max_unchanged_steps: 1,
        ..ControlLimits::default()
    };
    let browser = FakeBrowser::new([snapshot(), snapshot()]);
    let result = controller()
        .with_limits(limits)
        .run_with(
            &browser,
            &FakeDecisions::new([decision(
                Operation::Click,
                Some(element("e2", "button", "Search")),
            )]),
            &SessionId::new("session"),
            &TaskRequest::new("search"),
        )
        .await
        .expect("task result");

    assert_eq!(result.status, TaskStatus::Stuck);
    assert_eq!(browser.actions.lock().expect("actions").len(), 1);
}

#[tokio::test]
async fn a_changed_last_step_exhausts_the_budget() {
    let before = snapshot();
    let mut after = snapshot();
    after.tree.push_str("\nlink \"new\" @e9");
    let result = controller()
        .with_limits(ControlLimits {
            max_steps: 1,
            ..ControlLimits::default()
        })
        .run_with(
            &FakeBrowser::new([before, after]),
            &FakeDecisions::new([decision(Operation::ScrollDown, None)]),
            &SessionId::new("session"),
            &TaskRequest::new("find more"),
        )
        .await
        .expect("task result");

    assert_eq!(result.status, TaskStatus::Budget);
    assert!(result.steps[0].page_changed);
}

#[tokio::test]
async fn recoverable_browser_errors_are_traced_and_reobserved() {
    let mut done = decision(Operation::Done, None);
    done.goal_done = 0.9;
    let browser = FakeBrowser::with_outcomes(
        [snapshot(), snapshot()],
        [Err(tinybrowser::Error::not_actionable("covered"))],
    );
    let result = controller()
        .run_with(
            &browser,
            &FakeDecisions::new([decision(Operation::ScrollDown, None), done]),
            &SessionId::new("session"),
            &TaskRequest::new("continue"),
        )
        .await
        .expect("task result");

    assert_eq!(result.status, TaskStatus::Done);
    assert!(matches!(
        result.steps[0].outcome,
        StepOutcome::RecoverableError { .. }
    ));
}

#[tokio::test]
async fn nonrecoverable_browser_errors_retain_their_source() {
    let browser = FakeBrowser::with_outcomes(
        [snapshot()],
        [Err(tinybrowser::Error::browser_unavailable(
            "browser rejected action",
        ))],
    );
    let error = controller()
        .run_with(
            &browser,
            &FakeDecisions::new([decision(Operation::ScrollDown, None)]),
            &SessionId::new("session"),
            &TaskRequest::new("continue"),
        )
        .await
        .expect_err("page error is terminal");

    assert!(matches!(error, Error::Browser { .. }));
}

#[tokio::test]
async fn public_entrypoint_rejects_limits_before_touching_the_browser() {
    let error = controller()
        .with_limits(ControlLimits {
            max_steps: 0,
            ..ControlLimits::default()
        })
        .run(
            &tinybrowser::Browser::new(),
            &SessionId::new("missing"),
            &TaskRequest::new("anything"),
        )
        .await
        .expect_err("invalid limits");
    assert!(matches!(error, Error::InvalidTask { .. }));
}

#[tokio::test]
async fn decide_rejects_an_empty_goal_without_a_provider_call() {
    let error = controller()
        .decide(&TaskRequest::new("  "), &snapshot(), &[])
        .await
        .expect_err("blank goal");
    assert!(matches!(error, Error::InvalidTask { .. }));
}

#[test]
fn every_invalid_limit_and_input_name_is_rejected() {
    let control = controller();
    let task = TaskRequest::new("goal");
    for limits in [
        ControlLimits {
            max_steps: 0,
            ..ControlLimits::default()
        },
        ControlLimits {
            max_unchanged_steps: 0,
            ..ControlLimits::default()
        },
        ControlLimits {
            completion_threshold: 1.1,
            ..ControlLimits::default()
        },
    ] {
        assert!(control.clone().with_limits(limits).validate(&task).is_err());
    }
    let blank_input = TaskRequest::new("goal")
        .with_inputs(BTreeMap::from([(" ".to_owned(), "value".to_owned())]));
    assert!(control.validate(&blank_input).is_err());
}

#[test]
fn provider_failures_convert_without_losing_measurements() {
    let failure = tinyjevclient::EvaluationFailure {
        error: tinyjevclient::Error::Timeout,
        attempts: 2,
        latency: Duration::from_millis(10),
    };
    let error: Error = failure.into();
    let Error::Provider { source } = error else {
        panic!("provider variant");
    };
    assert_eq!(source.attempts, 2);
}
