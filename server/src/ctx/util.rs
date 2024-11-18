

// paths
pub fn query(p: &str) -> String {
    "/query/".to_owned() + p
}

pub fn create(p: &str) -> String {
    "/create/".to_owned() + p
}

pub fn submit(p: &str) -> String {
    "/submit/".to_owned() + p
}

pub fn operate(p: &str) -> String {
    "/operate/".to_owned() + p
}

pub fn util(p: &str) -> String {
    "/util/".to_owned() + p
}



/**************************/


pub type JsonObject = HashMap::<&'static str, Value>;



pub fn html_headers() -> HeaderMap {
    let mut headers = HeaderMap::new();
    headers.insert(header::CONTENT_TYPE, "text/html; charset=utf-8".parse().unwrap());
    headers
}

pub fn json_headers() -> HeaderMap {
    let mut headers = HeaderMap::new();
    headers.insert(header::CONTENT_TYPE, "application/json".parse().unwrap());
    headers
}

pub fn api_error(errmsg: &str) -> (HeaderMap, String) {
    (json_headers(), json!({"ret":1,"err":errmsg}).to_string())
}

pub fn api_ok() -> (HeaderMap, String){
    (json_headers(), json!({"ret":0,"ok":true}).to_string())
}

pub fn api_data_list(jsdts: Vec<Value>) -> (HeaderMap, String){
    let list = jsdts.iter().map(|a|a.to_string()).collect::<Vec<String>>().join(",");
    (json_headers(), format!(r#"{{"ret":0,"list":[{}]}}"#, list))
}

pub fn api_data(jsdts: HashMap<&'static str, Value>) -> (HeaderMap, String){
    let resjson = jsdts.iter().map(|(k,v)|
        format!(r#""{}":{}"#, k, v.to_string())
    ).collect::<Vec<String>>().join(",");
    (json_headers(), format!(r#"{{"ret":0,{}}}"#, resjson))
}


/**************************/


#[macro_export]
macro_rules! jsondata{
    ( $( $key: expr, $dv: expr,)+ ) => (
        {
            let mut data = JsonObject::new();
            $(
                data.insert($key, json!($dv));
            )+
            data
        }
    )
}


/**************************/


// auto drop <= 0
pub fn get_id_range(max: i64, page: i64, limit: i64, _instart: i64, decs: bool) -> Vec<i64> {
    let mut start = 1;
    if decs {
        start = max;
    }
    if page > 1 {
        if decs {
            start -= (page - 1) * limit;
        }else{
            start += (page - 1) * limit;
        }
    }
    let mut end = start + limit;
    if decs {
        end = start - limit;
    }
    // rev
    let mut rng: Vec<_> = (start..end).collect();
    if decs {
        rng = (end+1..start+1).rev().collect();
    }
    // ok
    rng.retain(|&x| x>=1 || x<=max);
    rng
}

