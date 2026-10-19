//! Lossless non-finite numbers at the native JSON/script boundary.
use crate::json as codec;
use rquickjs::{Array, Ctx, Function, Value};

pub(crate) fn project<'js>(ctx: &Ctx<'js>, value: Value<'js>) -> rquickjs::Result<serde_json::Value> {
    let stringify: Function = ctx.eval(r#"value => {
      const paths = [], parents = new WeakMap(); let root = true;
      const text = JSON.stringify(value, function(key, field) {
        const path = root ? [] : [...parents.get(this), key]; root = false;
        if (field && typeof field === 'object') parents.set(field, path);
        if (typeof field === 'number' && (!Number.isFinite(field) || Object.is(field, -0))) paths.push([path, Object.is(field, -0) ? '-0' : String(field)]);
        return field;
      });
      return [text, JSON.stringify(paths)];
    }"#)?;
    let result: Array = stringify.call((value,))?;
    let text: String = result.get(0)?;
    let paths: String = result.get(1)?;
    let mut value=codec::parse(&text).map_err(|error|rquickjs::Exception::throw_message(ctx,&error.to_string()))?;
    let paths=codec::parse(&paths).map_err(|error|rquickjs::Exception::throw_message(ctx,&error.to_string()))?;
    for entry in paths.as_array().unwrap() {
        let mut target=&mut value;
        for key in entry[0].as_array().unwrap() {
            let key=key.as_str().unwrap();
            target=if target.is_array() {target.get_mut(key.parse::<usize>().unwrap()).unwrap()}else{target.get_mut(key).unwrap()};
        }
        *target=codec::numeric(match entry[1].as_str().unwrap() {"Infinity"=>f64::INFINITY,"-Infinity"=>f64::NEG_INFINITY,"-0"=>-0.0,_=>f64::NAN});
    }
    Ok(value)
}

pub(crate) fn decode<'js>(ctx: &Ctx<'js>, value: &serde_json::Value) -> rquickjs::Result<Value<'js>> {
    if let Some(number)=codec::nonfinite(value).or_else(||value.as_f64().filter(|number| *number==0.0 && number.is_sign_negative())) {return Ok(Value::new_float(ctx.clone(),number));}
    let result=ctx.json_parse(codec::stringify(value))?;
    let assign:Function=ctx.eval("(root,path,value) => { let target=root; for(let i=0;i<path.length-1;i++) target=target[path[i]]; target[path[path.length-1]]=value; }")?;
    fn visit<'js>(ctx:&Ctx<'js>,assign:&Function<'js>,root:&Value<'js>,value:&serde_json::Value,path:&mut Vec<String>) -> rquickjs::Result<()> {
        if let Some(number)=codec::nonfinite(value).or_else(||value.as_f64().filter(|number| *number==0.0 && number.is_sign_negative())) {
            assign.call::<_,()>((root.clone(),ctx.json_parse(codec::stringify(&serde_json::json!(path)))?,number))?;
        } else if let Some(values)=value.as_array() {
            for (index,value) in values.iter().enumerate() {path.push(index.to_string());visit(ctx,assign,root,value,path)?;path.pop();}
        } else if let Some(values)=value.as_object() {
            for (key,value) in values {path.push(key.clone());visit(ctx,assign,root,value,path)?;path.pop();}
        }
        Ok(())
    }
    visit(ctx,&assign,&result,value,&mut Vec::new())?;
    Ok(result)
}
