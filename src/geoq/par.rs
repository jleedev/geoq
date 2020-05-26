use crate::geoq::{
    entity::{self, Entity},
    error::Error,
    input, reader,
};
use num_cpus;
use std::io;
use std::{
    io::BufRead,
    sync::{
        mpsc::{sync_channel, Receiver, RecvError, SyncSender},
        Arc,
    },
    thread::{self, JoinHandle},
};

enum WorkerInput {
    Item(String),
    Done,
}

enum WorkerOutput {
    Item(Result<Vec<String>, Error>),
    Done,
}

pub struct LineReader<'a> {
    reader: &'a mut dyn BufRead,
}

impl<'a> LineReader<'a> {
    pub fn new(reader: &'a mut dyn BufRead) -> LineReader<'a> {
        LineReader { reader }
    }
}

impl<'a> Iterator for LineReader<'a> {
    type Item = String;

    fn next(&mut self) -> Option<Self::Item> {
        reader::read_line(self.reader)
    }
}

// fn handle_line<F>(line: String, handler: F) -> Result<(), Error>
// where F: Fn(Entity) -> Result<(), Error>
// {
//     let input = try!(input::read_line(line));
//     let entities = try!(entity::from_input(input));
//     for e in entities {
//         try!(handler(e));
//     }
//     Ok(())
// }

pub fn for_stdin_entity<F: 'static>(handler: F) -> Result<(), Error>
where
    F: Send + Sync + Fn(Entity) -> Result<Vec<String>, Error>,
{
    let stdin = io::stdin();
    let mut stdin_reader = stdin.lock();
    for_entity_par(&mut stdin_reader, handler)
}

const WORKER_BUF_SIZE: usize = 5000;
pub fn for_entity_par<'a, F: 'static>(input: &'a mut dyn BufRead, handler: F) -> Result<(), Error>
where
    F: Send + Sync + Fn(Entity) -> Result<Vec<String>, Error>,
{
    let num_workers = num_cpus::get();
    let mut input_channels: Vec<SyncSender<WorkerInput>> = vec![];
    let mut threads: Vec<JoinHandle<_>> = vec![];
    let mut output_channels: Vec<Receiver<WorkerOutput>> = vec![];
    let handler_arc = Arc::new(handler);

    (0..num_workers).for_each(|_| {
        let (input_sender, input_receiver) = sync_channel(WORKER_BUF_SIZE);
        let (output_sender, output_receiver) = sync_channel(WORKER_BUF_SIZE);

        let handler = handler_arc.clone();

        let t = thread::spawn(move || {
            loop {
                match input_receiver.recv() {
                    Err(RecvError) => continue,
                    Ok(WorkerInput::Item(line)) => {
                        // TODO figure out how to make this work with arc
                        // output_sender.send(WorkerOutput::Item(handle_line(line, *handler)));

                        match input::read_line(line) {
                            Err(e) => output_sender.send(WorkerOutput::Item(Err(e))).unwrap(),
                            Ok(input) => match entity::from_input(input) {
                                Err(e) => output_sender.send(WorkerOutput::Item(Err(e))).unwrap(),
                                Ok(entities) => {
                                    let mut results = Vec::new();
                                    for e in entities {
                                        match handler(e) {
                                            Err(e) => {
                                                output_sender
                                                    .send(WorkerOutput::Item(Err(e)))
                                                    .unwrap();
                                                break;
                                            }
                                            Ok(lines) => results.extend(lines),
                                        }
                                    }
                                    output_sender.send(WorkerOutput::Item(Ok(results))).unwrap();
                                }
                            },
                        }
                    }
                    Ok(WorkerInput::Done) => {
                        output_sender.send(WorkerOutput::Done).unwrap();
                        break;
                    }
                }
            }
        });

        input_channels.push(input_sender);
        output_channels.push(output_receiver);
        threads.push(t);
    });

    let printer_thread = thread::spawn(move || {
        while !output_channels.is_empty() {
            for i in 0..output_channels.len() {
                let output = output_channels[i].recv();
                match output {
                    Err(RecvError) => continue,
                    Ok(WorkerOutput::Item(Ok(lines))) => {
                        for l in lines {
                            println!("{}", l);
                        }
                    }
                    Ok(WorkerOutput::Item(Err(e))) => {
                        eprintln!("Application error: {:?}", e);
                        ::std::process::exit(1);
                    }
                    Ok(WorkerOutput::Done) => {
                        output_channels.remove(i);
                        break;
                    }
                }
            }
        }
    });

    let reader = LineReader::new(input);
    for (i, line) in reader.enumerate() {
        input_channels[i % num_workers]
            .send(WorkerInput::Item(line))
            .unwrap();
    }
    (0..num_workers).for_each(|i| input_channels[i].send(WorkerInput::Done).unwrap());

    printer_thread
        .join()
        .expect("Couldn't wait for printer thread to complete");

    Ok(())
}

#[cfg(test)]
mod tests {
    use crate::geoq::par::for_entity_par;

    #[test]
    fn test_par_entities() {
        // Problem:
        // Outputs need to be processed by the single printer
        // round-robin to preserve ordering
        // But each input can potentially produce
        // many outputs
        // So outputs need to be Result(Vec<String>, Error)
        // and printer has to round-robin and then print all
        // outputs from that batch before continuing
        let mut input = r#"34.2277,-118.2623
{"type":"Polygon","coordinates":[[[-117.87231445312499,34.77997173591062],[-117.69653320312499,34.77997173591062],[-117.69653320312499,34.90170042871546],[-117.87231445312499,34.90170042871546],[-117.87231445312499,34.77997173591062]]]}
{"type":"Polygon","coordinates":[[[-118.27880859375001,34.522398580663314],[-117.89154052734375,34.522398580663314],[-117.89154052734375,34.649025753526985],[-118.27880859375001,34.649025753526985],[-118.27880859375001,34.522398580663314]]]}
"#.as_bytes();

        // let mut input = "9q5\n9q4".as_bytes();
        let res = for_entity_par(&mut input, move |entity| {
            Ok(vec![format!("handling entity {}", entity).to_owned()])
        });
        assert!(res.is_ok());
    }
}
