#[macro_use]
extern crate log;
#[macro_use]
extern crate nickel;
extern crate plugin;
extern crate typemap;
extern crate hyper;
extern crate uosql;
extern crate rustc_serialize;
extern crate cookie;
extern crate url;
extern crate server;

use uosql::Connection;
use std::io::Read;
use uosql::Error;
use std::collections::HashMap;
use std::collections::hash_map::Entry;
use std::ops::DerefMut;
use std::sync::{Arc, Mutex};
use plugin::Extensible;
use hyper::header::{Cookie, SetCookie};
use nickel::{Nickel, HttpRouter};
use cookie::Cookie as CookiePair;
use hyper::method::Method;
use url::form_urlencoded as urlencode;
use std::net::Ipv4Addr;
use std::str::FromStr;
use nickel::QueryString;
use server::storage::Rows;
use std::cmp::{max, min};

// Dummy key for typemap
struct ConnKey;
impl typemap::Key for ConnKey {
    type Value = Arc<Mutex<Connection>>;
}

#[derive(Debug)]
struct Login {
    user : String,
    password: String
}

/// Web based client
fn main() {

    let mut server = Nickel::new();
    let map: HashMap<String, Arc<Mutex<Connection>>>= HashMap::new();
    let map = Arc::new(Mutex::new(map));
    let map2 = map.clone();

    // Cookie managing
    server.utilize(middleware! { |req, res|

        // If login data has been posted, continue
        if req.origin.method == Method::Post {
            return Ok(nickel::Action::Continue(res));
        }

        // Look for session string in Cookies
        let sess = match req.origin.headers.get::<Cookie>() {
            // If no Cookie found, go to Login
            None => {
                let m = HashMap::<i8, i8>::new();
                return res.render("src/webclient/templates/login.tpl", &m);
            }
            // If there is a Cookie, eat it
            // (or find the matching UosqlDB-Cookie and extract session string)
            Some(cs) => {
                if let Some(sess) = cs.to_cookie_jar(&[1u8]).find("UosqlDB") {
                    sess.value
                // There is a cookie, but it is not ours :'(
                // Return to Login
                } else {
                    let m = HashMap::<i8, i8>::new();
                    return res.render("src/webclient/templates/login.tpl", &m);
                }
            },
        };

        // We have a session string and look for the matching connection in
        // our Session-Connection map
        let guard = map.lock().unwrap();
        match guard.get(&sess) {
            // No matching session: Old cookie
            None => {
                let mut data = HashMap::new();
                data.insert("err_msg", "Invalid Session");
                return res.render("src/webclient/templates/login.tpl", &data);
            }
            // There is a connection, we are logged in, we can enter the site!
            Some(con) => {
                req.extensions_mut().insert::<ConnKey>(con.clone());
                return Ok(nickel::Action::Continue(res));
            }
        }
    });

    // Login managing
    server.post("/login", middleware! { |req, mut res|

        // Read the post data
        let mut login_data = String::new();
        let read = req.origin.read_to_string(&mut login_data).unwrap();

        // Not sufficiently filled in, return to Login with error msg
        if read < 15 {
            let mut data = HashMap::new();
            data.insert("err_msg", "No data given");
            return res.render("src/webclient/templates/login.tpl", &data);
        }

        // Extract login data from Post string
        let pairs = urlencode::parse(login_data.as_bytes());
        let username = pairs.iter().find(|e| e.0 == "user").map(|e| e.1.clone());
        let password = pairs.iter().find(|e| e.0 == "password").map(|e| e.1.clone());
        let bind_in = pairs.iter().find(|e| e.0 == "bind").map(|e| e.1.clone());
        let port_in = pairs.iter().find(|e| e.0 == "port").map(|e| e.1.clone());

        // If eihter username or password are empty, return to Login page
        if username.is_none() || password.is_none()  {
            let mut data = HashMap::new();
            data.insert("err_msg", "Not all required fields given");
            return res.render("src/webclient/templates/login.tpl", &data);
        }

        let mut connection = "127.0.0.1".to_string();
        // Bind_in is never none, for inexplicable reasons
        if bind_in.clone().unwrap().len() > 8 {
            connection = bind_in.unwrap();
            test_bind(&connection);
        }

        let port = port_in.unwrap_or("4242".into()).parse::<u16>().unwrap_or(4242);

        // build Login struct
        let login = Login {
            user: username.unwrap(),
            password: password.unwrap()
        };

        // Generate new session string
        let sess_str = login.user.clone(); // Dummy

        // Try connect to db server
        // Insert connection and session string into hashmap
        let mut guard = map2.lock().unwrap();

        // create new connections
        match guard.deref_mut().entry(sess_str.clone()) {
            Entry::Occupied(_) => {},
            Entry::Vacant(v) => {
                let cres = Connection::connect(connection, port,
                                               login.user.clone(), login.password.clone());
                match cres {
                    Err(e) => {
                        let errstr = match e {
                            // Connection error handling
                            // TO DO: Wait for Display/Debug
                            Error::AddrParse(_) => {
                                "Could not connect to specified server."
                            },
                            Error::Io(_) => {
                                "Connection failure. Try again later."
                            },
                            Error::Decode(_) => {
                                "Could not readfsdfd data from server."
                            },
                            Error::Encode(_) => {
                                "Could not send data to server."
                            },
                            Error::UnexpectedPkg => {
                                "Unexpected Package."
                            },
                            Error::Auth => {
                                "Authentication failed."
                            },
                            Error::Server(_) => {
                                "Network Error."
                            },
                        };
                        let mut data = HashMap::new();
                        data.insert("err", errstr);
                        return res.render("src/webclient/templates/error.tpl", &data);
                    }
                    Ok(c) => {
                        v.insert(Arc::new(Mutex::new(c)));
                    },
                }
            }
        };

        // Set a Cookie with the session string as its value
        // sess_str is set to a value here, so we can safely unwrap
        let keks = CookiePair::new("UosqlDB".to_owned(), sess_str.clone());
        res.headers_mut().set(SetCookie(vec![keks]));

        // Redirect to the greeting page
        *res.status_mut() = nickel::status::StatusCode::Found;
        res.headers_mut().set_raw("location", vec![b"/".to_vec()]);
        return res.send("");
    });

    // Disconnect from server
    server.get("/logout", middleware! { |req, mut res|

        let mut con = req.extensions().get::<ConnKey>().unwrap().lock().unwrap();
        let mut data = HashMap::new();

        data.insert("name", con.get_username().to_string());

        match con.quit(){
            Ok(_) => { },
            Err(_) => error!("Connection could not be quit."),
        }

        // Remove Cookie
        match req.origin.headers.get::<Cookie>() {

            None => { }
            Some(cs) => {
                let cj = cs.to_cookie_jar(&[1u8]);
                cj.remove("UosqlDB");
                res.headers_mut().set(SetCookie::from_cookie_jar(&cj));
            },
        };

        return res.render("src/webclient/templates/logout.tpl", &data);
    });

        // Greeting page
    server.get("/", middleware! { |req, res|

        // Look for connection
        let tmp = req.extensions().get::<ConnKey>().unwrap().clone();
        let mut con = tmp.lock().unwrap();

        let mut data = HashMap::new();

        let query = req.query().get("sql");
        if !query.is_none() {
            let result = match con.execute(query.unwrap().to_string()) {
                Ok(r) => r,
                Err(e) => {
                    let errstr = match e {
                        Error::Io(_) => "Connection failure. Try again later.",
                        Error::Decode(_) => "Could not read data from server.",
                        Error::Encode(_) => "Could not send data to server.",
                        Error::UnexpectedPkg => "Received unexpected package.",
                        Error::Server(_) => "Server error.",
                        _ => "Unexpected behaviour during execute().",
                    };
                    let mut data = HashMap::new();
                    data.insert("err", errstr);
                    return res.render("src/webclient/templates/error.tpl", &data);
                }
            };
            // let s = format!("{:?}", result);
            // data.insert("result", s.to_string());
            let res_output = display(&result);
            println!("{}", res_output);
            data.insert("result", res_output);
        }

        // Current display with short welcome message
        let version = con.get_version().to_string();
        let port = con.get_port().to_string();

        data.insert("name", con.get_username().to_string());
        data.insert("version", version);
        data.insert("bind", con.get_ip().to_string());
        data.insert("port", port);
        data.insert("msg", con.get_message().to_string());
        return res.render("src/webclient/templates/main.tpl", &data);
    });

    server.listen("127.0.0.1:6767");
}

fn test_bind (bind : &str) -> bool {
    let result = match Ipv4Addr::from_str(bind) {
        Ok(_) => true,
        Err(_) => {
            false
        }
    };
    result
}

fn display (rows: &Rows) -> String {
    if rows.data.is_empty() {
        display_meta(&rows)
    } else {
        display_data(&rows)
    }
}

fn display_data(row: &Rows) -> String {

    let mut result = String::new();

    let mut cols = vec![];
    for i in &row.columns {
        cols.push(max(12, i.name.len()));
    }

    // column names
    result.push_str(&display_seperator(&cols));

    for i in 0..(cols.len()) {
        if row.columns[i].name.len() > 27 {
            result.push_str(&format!("| {}... ", &row.columns[i].name[..27]));
        } else {
            result.push_str(&format!("| {1: ^0$} ", min(30, cols[i]), row.columns[i].name));
        }
    }
    result.push_str("|\n");

    result.push_str(&display_seperator(&cols));

    result
}

fn display_meta(row: &Rows) -> String{

    let mut result = String::new();
    result.trim();

    // print meta data
    let mut cols = vec![];
    for i in &row.columns {
        cols.push(max(12, max(i.name.len(), i.description.len())));
    }

    // // Column name +---
    result.push_str("\n");
    result.push_str("+");
    let col_name = "Column name";
    for _ in 0..(col_name.len()+2) {
        result.push_str("-");
    }

    // for every column +---
    result.push_str(&display_seperator(&cols));

    result.push_str(&format!("| {} ", col_name));
    // name of every column
    for i in 0..(cols.len()) {
        if row.columns[i].name.len() > 27 {
            result.push_str(&format!("| {}... ", &row.columns[i].name[..27]));
        } else {
            result.push_str(&format!("| {1: ^0$} ", min(30, cols[i]), row.columns[i].name));
        }
    }
    result.push_str("|\n");

    // format +--
    result.push_str("+");
    for _ in 0..(col_name.len()+2) {
        result.push_str("-");
    }

    result.push_str(&display_seperator(&cols));

    result.push_str(&format!("| {1: <0$} ", col_name.len(), "Type"));
    for i in 0..(cols.len()) {
        result.push_str(&format!("| {1: ^0$} ", min(30, cols[i]), format!("{:?}", row.columns[i].sql_type)));
    }
    result.push_str("|\n");

    result.push_str(&format!("| {1: <0$} ", col_name.len(), "Primary"));
    for i in 0..(cols.len()) {
        result.push_str(&format!("| {1: ^0$} ", min(30, cols[i]), row.columns[i].is_primary_key));
    }
    result.push_str("|\n");

    result.push_str(&format!("| {1: <0$} ", col_name.len(), "Allow NULL"));
    for i in 0..(cols.len()) {
        result.push_str(&format!("| {1: ^0$} ", min(30, cols[i]), row.columns[i].allow_null));
    }
    result.push_str("|\n");

    result.push_str(&format!("| {1: <0$} ", col_name.len(), "Description"));
    for i in 0..(cols.len()) {
        if row.columns[i].description.len() > 27 {
            //splitten
            result.push_str(&format!("| {}... ", &row.columns[i].description[..27]));
        } else {
            result.push_str("FALSE");
            result.push_str(&format!("| {1: ^0$} ", min(30, cols[i]), row.columns[i].description));
        }
    }
    result.push_str("|\n");

    result.push_str("+");
    for _ in 0..(col_name.len()+2) {
        result.push_str("-");
    }

    result.push_str(&display_seperator(&cols));
    result
}

pub fn display_seperator(cols: &Vec<usize>) -> String{
    let mut res = String::new();
    for i in 0..(cols.len()) {
        res.push_str("+--");
        for _ in 0..min(30, cols[i]) {
            res.push_str("-");
        }
    }
    res.push_str("+\n");
    res
}
