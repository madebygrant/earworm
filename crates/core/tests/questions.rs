// Its own test binary, so its own process: `cancel_open_questions` reaches every question in the
// process, and inside the unit tests it would cancel other tests' questions mid-run.
use earworm_core::protocol::{Reply, answer, cancel_open_questions, open_question};

#[test]
fn a_front_end_that_lost_its_ids_can_cancel_everything_open() {
    let (first, first_rx) = open_question();
    let (second, second_rx) = open_question();
    cancel_open_questions();
    assert!(matches!(first_rx.recv(), Ok(Reply::Cancel)), "the first question was left waiting");
    assert!(matches!(second_rx.recv(), Ok(Reply::Cancel)), "the second question was left waiting");
    assert!(!answer(first, Reply::Choice(0)), "a cancelled question took an answer");
    assert!(!answer(second, Reply::Choice(0)));

    // The registry still works after, for the session's next question.
    let (third, third_rx) = open_question();
    assert!(answer(third, Reply::Choice(1)));
    assert!(matches!(third_rx.recv(), Ok(Reply::Choice(1))));
}
