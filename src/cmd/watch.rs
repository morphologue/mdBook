use crate::{get_book_dir, open};
use clap::{App, ArgMatches, SubCommand};
use mdbook::errors::Result;
use mdbook::utils;
use mdbook::MDBook;
use notify::Watcher;
use std::path::{Path, PathBuf};
use std::sync::mpsc::channel;
use std::thread::sleep;
use std::time::Duration;

// Create clap subcommand arguments
pub fn make_subcommand<'a, 'b>() -> App<'a, 'b> {
    SubCommand::with_name("watch")
        .about("Watches a book's files and rebuilds it on changes")
        .arg_from_usage(
            "-d, --dest-dir=[dest-dir] 'Output directory for the book{n}\
             Relative paths are interpreted relative to the book's root directory.{n}\
             If omitted, mdBook uses build.build-dir from book.toml or defaults to `./book`.'",
        )
        .arg_from_usage(
            "[dir] 'Root directory for the book{n}\
             (Defaults to the Current Directory when omitted)'",
        )
        .arg_from_usage("-o, --open 'Open the compiled book in a web browser'")
}

// Watch command implementation
pub fn execute(args: &ArgMatches) -> Result<()> {
    let book_dir = get_book_dir(args);
    let book = MDBook::load(&book_dir)?;

    if args.is_present("open") {
        book.build()?;
        open(book.build_dir_for("html").join("index.html"));
    }

    trigger_on_change(&book, |paths, book_dir| {
        info!("Files changed: {:?}\nBuilding book...\n", paths);
        let result = MDBook::load(&book_dir).and_then(|b| b.build());

        if let Err(e) = result {
            error!("Unable to build the book");
            utils::log_backtrace(&e);
        }
    });

    Ok(())
}

fn remove_ignored_files(book_root: &PathBuf, paths: &[PathBuf]) -> Vec<PathBuf> {
    if paths.is_empty() {
        return vec![];
    }

    let gitignore_path = book_root.with_file_name(".gitignore");

    match gitignore::File::new(gitignore_path.as_path()) {
        Ok(exclusion_checker) => paths
            .iter()
            .filter(|path| match exclusion_checker.is_excluded(path) {
                Ok(exclude) => !exclude,
                Err(error) => {
                    warn!(
                        "Unable to determine if {:?} is excluded: {:?}. Including it.",
                        &path, error
                    );
                    true
                }
            })
            .map(|path| path.to_path_buf())
            .collect(),
        Err(error) => {
            warn!(
                "Unable to read gitignore file at {:?} file: {:?}. All files will be allowed.",
                gitignore_path, error
            );
            paths.iter().map(|path| path.to_path_buf()).collect()
        }
    }
}

/// Calls the closure when a book source file is changed, blocking indefinitely.
pub fn trigger_on_change<F>(book: &MDBook, closure: F)
where
    F: Fn(Vec<PathBuf>, &Path),
{
    use notify::DebouncedEvent::*;
    use notify::RecursiveMode::*;

    // Create a channel to receive the events.
    let (tx, rx) = channel();

    let mut watcher = match notify::watcher(tx, Duration::from_secs(1)) {
        Ok(w) => w,
        Err(e) => {
            error!("Error while trying to watch the files:\n\n\t{:?}", e);
            std::process::exit(1)
        }
    };

    // Add the source directory to the watcher
    if let Err(e) = watcher.watch(book.source_dir(), Recursive) {
        error!("Error while watching {:?}:\n    {:?}", book.source_dir(), e);
        std::process::exit(1);
    };

    let _ = watcher.watch(book.theme_dir(), Recursive);

    // Add the book.toml file to the watcher if it exists
    let _ = watcher.watch(book.root.join("book.toml"), NonRecursive);

    info!("Listening for changes...");

    loop {
        let first_event = rx.recv().unwrap();
        sleep(Duration::from_millis(50));
        let other_events = rx.try_iter();

        let all_events = std::iter::once(first_event).chain(other_events);

        let paths = all_events
            .filter_map(|event| {
                debug!("Received filesystem event: {:?}", event);

                match event {
                    Create(path) | Write(path) | Remove(path) | Rename(_, path) => Some(path),
                    _ => None,
                }
            })
            .collect::<Vec<_>>();

        let paths = remove_ignored_files(&book.root, &paths[..]);

        if !paths.is_empty() {
            closure(paths, &book.root);
        }
    }
}
