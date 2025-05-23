use std::{
    cell::RefCell,
    fs::File,
    io::Read,
    path::PathBuf,
    str::FromStr,
    time::{Duration, Instant},
};

const INITIAL_READ_BUFFER_CAPACITY: usize = 128;

pub struct FileSource<T: FromStr + Clone, const REQUIRED: bool> {
    filepath: PathBuf,
    value: RefCell<Option<T>>,
    refresh_interval: Option<Duration>,
    last_refresh: RefCell<Option<Instant>>,
    auto_trim: bool,
}

pub trait ValueSource<T, E: std::fmt::Debug> {
    fn value(&self) -> Result<T, ValueError<E>>;
}

#[derive(thiserror::Error, Debug)]
pub enum RefreshFileSourceError<E: std::fmt::Debug> {
    #[error("error reading config from file: {0}")]
    IOError(#[from] std::io::Error),
    #[error("error parsing string value to type: {0}")]
    ParseError(E),
    #[error("no value given/file found")]
    NoValue,
}

impl<E: std::fmt::Debug, T: FromStr<Err = E> + Clone, const REQUIRED: bool>
    FileSource<T, REQUIRED>
{
    pub fn from_path(filepath: PathBuf) -> Self {
        Self {
            filepath,
            auto_trim: true,
            value: RefCell::new(None),
            refresh_interval: None,
            last_refresh: RefCell::new(None),
        }
    }

    pub fn set_refresh_interval(&mut self, interval: Option<Duration>) -> &mut Self {
        self.refresh_interval = interval;
        self
    }

    pub fn set_auto_trim(&mut self, auto_trim: bool) -> &mut Self {
        self.auto_trim = auto_trim;
        self
    }

    fn set_value(&self, value: Option<T>) -> () {
        *self.value.borrow_mut() = value;
        *self.last_refresh.borrow_mut() = Some(Instant::now());
    }

    pub fn refresh_on_timeout(&self) -> Result<(), RefreshFileSourceError<E>> {
        let last_refresh = self.last_refresh.borrow().to_owned();
        if last_refresh.is_none_or(|last_refresh| {
            self.refresh_interval
                .is_some_and(|refresh_interval| (last_refresh + refresh_interval) < Instant::now())
        }) {
            self.refresh_value()?;
        }

        Ok(())
    }

    pub fn refresh_value(&self) -> Result<(), RefreshFileSourceError<E>> {
        if !self.filepath.exists() {
            if REQUIRED {
                return Err(RefreshFileSourceError::NoValue);
            } else {
                self.set_value(None);
                return Ok(());
            }
        }

        let mut file = File::open(&self.filepath)?;
        let mut read_buf = String::with_capacity(INITIAL_READ_BUFFER_CAPACITY);
        let _read_bytes = file.read_to_string(&mut read_buf)?;

        let to_parse = if self.auto_trim {
            read_buf.trim()
        } else {
            read_buf.as_str()
        };

        let parsed = to_parse
            .parse::<T>()
            .map_err(|e| RefreshFileSourceError::ParseError(e))?;

        self.set_value(Some(parsed));
        Ok(())
    }
}

#[derive(thiserror::Error, Debug)]
pub enum ValueError<E: std::fmt::Debug> {
    #[error("no value given for required config variable")]
    NoValue,
    #[error("error refreshing values: {0}")]
    RefreshFileSourceError(#[from] RefreshFileSourceError<E>),
}

impl<E: std::fmt::Debug, T: FromStr<Err = E> + Clone> ValueSource<T, E> for FileSource<T, true> {
    fn value(&self) -> Result<T, ValueError<E>> {
        self.refresh_on_timeout()?;
        Ok(self.value.borrow().to_owned().ok_or(ValueError::NoValue)?)
    }
}

impl<E: std::fmt::Debug, T: FromStr<Err = E> + Clone> ValueSource<Option<T>, E>
    for FileSource<T, false>
{
    fn value(&self) -> Result<Option<T>, ValueError<E>> {
        self.refresh_on_timeout()?;
        Ok(self.value.borrow().to_owned())
    }
}

#[cfg(test)]
mod tests {
    use std::io::Write;

    use super::*;

    #[test]
    fn simple_required() {
        let mut source: FileSource<String, true> =
            FileSource::from_path("sources/test-required".into());

        assert_eq!(source.value().unwrap(), "hello world!");
    }

    #[test]
    fn simple_optional_given() {
        let mut source: FileSource<String, false> =
            FileSource::from_path("sources/test-optional".into());

        assert_eq!(
            source.value().unwrap(),
            Some("hello optional world!".to_owned())
        );
    }

    #[test]
    fn simple_optional_missing() {
        let mut source: FileSource<String, false> =
            FileSource::from_path("sources/test-optional-missing".into());

        assert_eq!(source.value().unwrap(), None);
    }

    #[test]
    fn timeout_refresh() {
        let file_path = "sources/refresh-overwrite";
        {
            let mut source_file = File::create(file_path).unwrap();
            source_file.write_all("first".as_bytes()).unwrap();
        }
        let mut source: FileSource<String, false> = FileSource::from_path(file_path.into());
        source.set_refresh_interval(Some(Duration::from_secs(5)));
        assert_eq!(source.value().unwrap(), Some("first".to_owned()));
        {
            let mut source_file = File::create(file_path).unwrap();
            source_file.write_all("second".as_bytes()).unwrap();
            source_file.flush().unwrap();
        }
        std::thread::sleep(Duration::from_secs(5));

        assert_eq!(source.value().unwrap(), Some("second".to_owned()));
    }
}
