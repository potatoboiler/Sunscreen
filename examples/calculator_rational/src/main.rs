mod error;

use std::io::{self, Write};
use std::sync::mpsc::{Receiver, Sender};
use std::thread::{self, JoinHandle};
use sunscreen::{
    fhe_program,
    types::{bfv::Rational, Cipher},
    Ciphertext, CompiledFheProgram, Compiler, Params, PlainModulusConstraint, PublicKey, Runtime,
    RuntimeError,
};

use crate::error::Error;

fn help() {
    println!("This is a privacy preserving calculator. You can add, subtract, multiply, divide decimal values. The operation is sent to Bob in cleartext while the operands
    are encrypted. Bob chooses an FHE program corresponding to the selected operation and computes the result. Additionally, Bob saves the last computed value as `ans`, which you may use as either operand.");
    println!("Since this example is to demo encryption, not parsing, you must insert exactly one space between the operand and values.");
    println!("Type exit to quit.");
    println!("Example:");
    println!(">> 3 + 6.5");
    println!("9.5");
    println!(">> ans / 5");
    println!("1.9");
    println!("");
}

enum Term {
    Ans,
    F64(f64),
    Encrypted(Ciphertext),
}

enum Operand {
    Add,
    Sub,
    Mul,
    Div,
}

struct Expression {
    left: Term,
    op: Operand,
    right: Term,
}

enum ParseResult {
    Help,
    Exit,
    Expression(Expression),
}

fn parse_input(line: &str) -> Result<ParseResult, Error> {
    if line == "help" {
        return Ok(ParseResult::Help);
    } else if line == "exit" {
        return Ok(ParseResult::Exit);
    }

    let mut terms = line.split(" ");

    let left = terms.next().ok_or(Error::ParseError)?;

    let left_term = if left == "ans" {
        Term::Ans
    } else {
        Term::F64(left.parse::<f64>().map_err(|_| Error::ParseError)?)
    };

    let operand = terms.next().ok_or(Error::ParseError)?;

    let operand = if operand == "+" {
        Operand::Add
    } else if operand == "-" {
        Operand::Sub
    } else if operand == "/" {
        Operand::Div
    } else if operand == "*" {
        Operand::Mul
    } else {
        return Err(Error::ParseError);
    };

    let right = terms.next().ok_or(Error::ParseError)?;

    let right_term = if right == "ans" {
        Term::Ans
    } else {
        Term::F64(right.parse::<f64>().map_err(|_| Error::ParseError)?)
    };

    Ok(ParseResult::Expression(Expression {
        left: left_term,
        op: operand,
        right: right_term,
    }))
}

fn encrypt_term(runtime: &Runtime, public_key: &PublicKey, input: Term) -> Result<Term, Error> {
    match input {
        Term::Ans => Ok(Term::Ans),
        Term::F64(v) => Ok(Term::Encrypted(
            runtime
                .encrypt(Rational::try_from(v)?, &public_key)?,
        )),
        _ => {
            panic!("This shouldn't happen.");
        }
    }
}

fn alice(
    send_pub: Sender<PublicKey>,
    send_calc: Sender<Expression>,
    recv_params: Receiver<Params>,
    recv_res: Receiver<Ciphertext>,
) -> JoinHandle<()> {
    thread::spawn(move || {
        let thread_body = move || -> Result<(), Error> {
            let stdin = io::stdin();
            let mut stdout = io::stdout();

            println!("Bob's private calculator. Type `help` for help.");

            // Bob needs to send us the scheme parameters compatible with his FHE program.
            let params = recv_params.recv()?;

            let runtime = Runtime::new(&params)?;

            let (public_key, private_key) = runtime.generate_keys()?;

            // Send Bob a copy of our public keys.
            send_pub.send(public_key.clone())?;

            loop {
                print!(">> ");

                stdout.flush()?;

                let mut line = String::new();
                stdin.read_line(&mut line)?;
                let line = line.trim();

                // Read the line and parse it into operands and an operator.
                let parsed = parse_input(&line);

                let Expression { left, right, op } = match parsed {
                    Ok(ParseResult::Expression(val)) => val,
                    Ok(ParseResult::Exit) => std::process::exit(0),
                    Ok(ParseResult::Help) => {
                        help();
                        continue;
                    }
                    Err(_) => {
                        println!("Parse error. Try again.");
                        continue;
                    }
                };

                // Encrypt the left and right terms.
                let encrypt_left = encrypt_term(&runtime, &public_key, left)?;
                let encrypt_right = encrypt_term(&runtime, &public_key, right)?;

                // Send Bob our encrypted operation.
                send_calc
                    .send(Expression {
                        left: encrypt_left,
                        right: encrypt_right,
                        op: op,
                    })?;

                // Get our result from Bob and print it.
                let result: Ciphertext = recv_res.recv()?;
                let result: Rational = match runtime.decrypt(&result, &private_key) {
                    Ok(v) => v,
                    Err(RuntimeError::TooMuchNoise) => {
                        println!("Decryption failed: too much noise");
                        continue;
                    }
                    Err(e) => { return Err(e.into()); },
                };
                let result: f64 = result.into();

                println!("{}", result);
            }
        };

        match thread_body() {
            Ok(_) => {},
            Err(e) => panic!("{:#?}", e),
        }
    })
}

fn compile_fhe_programs() -> Result<(
    CompiledFheProgram,
    CompiledFheProgram,
    CompiledFheProgram,
    CompiledFheProgram,
), Error> {
    #[fhe_program(scheme = "bfv")]
    fn add(a: Cipher<Rational>, b: Cipher<Rational>) -> Cipher<Rational> {
        a + b
    }

    #[fhe_program(scheme = "bfv")]
    fn sub(a: Cipher<Rational>, b: Cipher<Rational>) -> Cipher<Rational> {
        a - b
    }

    #[fhe_program(scheme = "bfv")]
    fn mul(a: Cipher<Rational>, b: Cipher<Rational>) -> Cipher<Rational> {
        a * b
    }

    #[fhe_program(scheme = "bfv")]
    fn div(a: Cipher<Rational>, b: Cipher<Rational>) -> Cipher<Rational> {
        a / b
    }

    // In order for ciphertexts to be compatible between FHE programs, they must all use the same
    // parameters.
    // With rational numbers, each of these FHE programs produces roughly the same amount of noise.
    // To be sure, we compile one of them with the default parameter search, and explicitly
    // pass these parameters when compiling the other FHE programs so they are compatible.
    let add_program = Compiler::with_fhe_program(add)
        // We need to make the noise margin large enough so we can do a few repeated calculations.
        .additional_noise_budget(32)
        .plain_modulus_constraint(PlainModulusConstraint::Raw(1_000_000))
        .compile()
        ?;

    let mul_program = Compiler::with_fhe_program(mul)
        .with_params(&add_program.metadata.params)
        .compile()
        ?;

    let div_program = Compiler::with_fhe_program(div)
        .with_params(&add_program.metadata.params)
        .compile()
        ?;

    let sub_program = Compiler::with_fhe_program(sub)
        .with_params(&add_program.metadata.params)
        .compile()
        ?;

    Ok((add_program, sub_program, mul_program, div_program))
}

fn bob(
    recv_pub: Receiver<PublicKey>,
    recv_calc: Receiver<Expression>,
    send_params: Sender<Params>,
    send_res: Sender<Ciphertext>,
) -> JoinHandle<()> {
    thread::spawn(move || {
        let thread_body = move || -> Result<(), Error> {
            let (add, sub, mul, div) = compile_fhe_programs()?;

            send_params.send(add.metadata.params.clone())?;

            let public_key = recv_pub.recv()?;

            let runtime = Runtime::new(&add.metadata.params)?;

            let mut ans = runtime
                .encrypt(Rational::try_from(0f64)?, &public_key)
                ?;

            loop {
                let Expression { left, right, op } = recv_calc.recv()?;

                let left = match left {
                    Term::Ans => ans.clone(),
                    Term::Encrypted(c) => c,
                    _ => panic!("Alice sent us a plaintext!"),
                };

                let right = match right {
                    Term::Ans => ans.clone(),
                    Term::Encrypted(c) => c,
                    _ => panic!("Alice sent us a plaintext!"),
                };

                let mut c = match op {
                    Operand::Add => runtime.run(&add, vec![left, right], &public_key)?,
                    Operand::Sub => runtime.run(&sub, vec![left, right], &public_key)?,
                    Operand::Mul => runtime.run(&mul, vec![left, right], &public_key)?,
                    Operand::Div => runtime.run(&div, vec![left, right], &public_key)?,
                };

                // Our FHE program produces a single value, so move the value out of the vector.
                let c = c.drain(0..).next().expect("Internal error: FHE program didn't produce a result");
                ans = c.clone();

                send_res.send(c)?;
            }

        };

        match thread_body() {
            Ok(_) => {},
            Err(e) => panic!("{:#?}", e),
        };
    })
}

fn main() -> Result<(), Error> {
    // A channel for Alice to send her public keys to Bob.
    let (send_alice_pub, receive_alice_pub) = std::sync::mpsc::channel::<PublicKey>();

    // A channel for Alice to send calculation requests to Bob.
    let (send_alice_calc, receive_alice_calc) = std::sync::mpsc::channel::<Expression>();

    // A channel for Bob to send scheme params to Alice
    let (send_bob_params, receive_bob_params) = std::sync::mpsc::channel::<Params>();

    // A channel for Bob to send calculation results to Alice.
    let (send_bob_result, receive_bob_result) = std::sync::mpsc::channel::<Ciphertext>();

    // We intentionally break Alice and Bob's roles into different functions to clearly
    // show the separation of their roles. In a real application, they're usually on
    // different machines communicating over a real protocol (e.g. TCP sockets).
    let a = alice(
        send_alice_pub,
        send_alice_calc,
        receive_bob_params,
        receive_bob_result,
    );
    let b = bob(
        receive_alice_pub,
        receive_alice_calc,
        send_bob_params,
        send_bob_result,
    );

    a.join().unwrap();
    b.join().unwrap();

    Ok(())
}
