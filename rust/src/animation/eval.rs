use anyhow::{bail, Result};
use std::collections::HashMap;

#[derive(Debug, Clone, PartialEq)]
enum Token {
    Number(f64), Identifier(String), Plus, Minus, Multiply, Divide, Modulo, Power,
    LeftParen, RightParen, Comma, End,
}

fn tokenize(input: &str) -> Result<Vec<Token>> {
    let bytes=input.as_bytes();
    let mut tokens=Vec::new();
    let mut i=0usize;
    while i<bytes.len() {
        let ch=bytes[i];
        if ch.is_ascii_whitespace() { i+=1; continue; }
        match ch {
            b'+'=>{tokens.push(Token::Plus);i+=1;}
            b'-'=>{tokens.push(Token::Minus);i+=1;}
            b'*'=>{if bytes.get(i+1)==Some(&b'*'){tokens.push(Token::Power);i+=2;}else{tokens.push(Token::Multiply);i+=1;}}
            b'/'=>{tokens.push(Token::Divide);i+=1;}
            b'%'=>{tokens.push(Token::Modulo);i+=1;}
            b'('=>{tokens.push(Token::LeftParen);i+=1;}
            b')'=>{tokens.push(Token::RightParen);i+=1;}
            b','=>{tokens.push(Token::Comma);i+=1;}
            _ if ch.is_ascii_digit()||ch==b'.'=>{
                let start=i;
                let mut exponent=false;
                while i<bytes.len(){
                    let c=bytes[i];
                    if c.is_ascii_digit()||c==b'.'{i+=1;}
                    else if (c==b'e'||c==b'E')&&!exponent{
                        exponent=true;i+=1;
                        if matches!(bytes.get(i),Some(b'+')|Some(b'-')){i+=1;}
                    }else{break;}
                }
                while matches!(bytes.get(i),Some(b'f')|Some(b'F')|Some(b'd')|Some(b'D')|Some(b'l')|Some(b'L')){i+=1;}
                let raw=&input[start..i];
                let value=raw.trim_end_matches(|c:char|matches!(c,'f'|'F'|'d'|'D'|'l'|'L')).parse::<f64>()?;
                tokens.push(Token::Number(value));
            }
            _ if ch.is_ascii_alphabetic()||ch==b'_'||ch==b'$'=>{
                let start=i;i+=1;
                while i<bytes.len()&&(bytes[i].is_ascii_alphanumeric()||matches!(bytes[i],b'_'|b'$'|b'.')){i+=1;}
                tokens.push(Token::Identifier(input[start..i].to_owned()));
            }
            _=>bail!("Unsupported token {:?} in animation expression",ch as char),
        }
    }
    tokens.push(Token::End);
    Ok(tokens)
}

struct Parser<'a>{tokens:&'a[Token],position:usize,environment:&'a HashMap<String,f64>}
impl<'a> Parser<'a>{
    fn peek(&self)->&Token{self.tokens.get(self.position).unwrap_or(&Token::End)}
    fn take(&mut self)->Token{
        let token=self.peek().clone();
        if token!=Token::End{self.position+=1;}
        token
    }
    fn consume(&mut self,expected:&Token)->bool{
        if self.peek()==expected{self.take();true}else{false}
    }
    fn parse(&mut self)->Result<f64>{
        let value=self.add_sub()?;
        if self.peek()!=&Token::End{bail!("Unexpected token {:?} in animation expression",self.peek());}
        if !value.is_finite(){bail!("Non-finite animation expression result");}
        Ok(value)
    }
    fn add_sub(&mut self)->Result<f64>{
        let mut value=self.multiply()?;
        loop{
            if self.consume(&Token::Plus){value+=self.multiply()?;}
            else if self.consume(&Token::Minus){value-=self.multiply()?;}
            else{break;}
        }
        Ok(value)
    }
    fn multiply(&mut self)->Result<f64>{
        let mut value=self.unary()?;
        loop{
            if self.consume(&Token::Multiply){value*=self.unary()?;}
            else if self.consume(&Token::Divide){
                let rhs=self.unary()?;
                if rhs.abs()<f64::EPSILON{bail!("Division by zero in animation expression");}
                value/=rhs;
            }
            else if self.consume(&Token::Modulo){
                let rhs=self.unary()?;
                if rhs.abs()<f64::EPSILON{bail!("Modulo by zero in animation expression");}
                value%=rhs;
            }
            else{break;}
        }
        Ok(value)
    }
    fn unary(&mut self)->Result<f64>{
        if self.consume(&Token::Plus){self.unary()}
        else if self.consume(&Token::Minus){Ok(-self.unary()?)}
        else{self.power()}
    }
    fn power(&mut self)->Result<f64>{
        let value=self.primary()?;
        if self.consume(&Token::Power){Ok(value.powf(self.unary()?))}else{Ok(value)}
    }
    fn primary(&mut self)->Result<f64>{
        match self.take(){
            Token::Number(value)=>Ok(value),
            Token::LeftParen=>{
                let value=self.add_sub()?;
                if !self.consume(&Token::RightParen){bail!("Unclosed parenthesis in animation expression");}
                Ok(value)
            }
            Token::Identifier(name)=>{
                if self.consume(&Token::LeftParen){
                    let mut args=Vec::new();
                    if !self.consume(&Token::RightParen){
                        loop{
                            args.push(self.add_sub()?);
                            if self.consume(&Token::RightParen){break;}
                            if !self.consume(&Token::Comma){bail!("Expected comma in animation function call");}
                        }
                    }
                    let function=name.rsplit('.').next().unwrap_or(&name);
                    match (function,args.as_slice()){
                        ("sin",[a])=>Ok(a.sin()),
                        ("cos",[a])=>Ok(a.cos()),
                        ("tan",[a])=>Ok(a.tan()),
                        ("abs",[a])=>Ok(a.abs()),
                        ("sqrt",[a]) if *a>=0.0=>Ok(a.sqrt()),
                        ("radians",[a])=>Ok(a.to_radians()),
                        ("degrees",[a])=>Ok(a.to_degrees()),
                        ("pow",[a,b])=>Ok(a.powf(*b)),
                        ("min",[a,b])=>Ok(a.min(*b)),
                        ("max",[a,b])=>Ok(a.max(*b)),
                        _=>bail!("Unsupported animation function {name}({} argument(s))",args.len()),
                    }
                }else{
                    match name.as_str(){
                        "PI"|"Math.PI"|"StrictMath.PI"|"Mth.PI"|"MathHelper.PI"=>Ok(std::f64::consts::PI),
                        "HALF_PI"|"Mth.HALF_PI"=>Ok(std::f64::consts::FRAC_PI_2),
                        "TWO_PI"|"Mth.TWO_PI"=>Ok(std::f64::consts::TAU),
                        _=>self.environment.get(&name).copied().ok_or_else(||anyhow::anyhow!("Unknown animation variable {name}")),
                    }
                }
            }
            token=>bail!("Unexpected token {token:?} in animation expression"),
        }
    }
}

/// Evaluate the expression subset emitted by the native JVM animation interpreter.
pub fn evaluate(expression:&str,environment:&HashMap<String,f64>)->Result<f64>{
    let tokens=tokenize(expression)?;
    Parser{tokens:&tokens,position:0,environment}.parse()
}

fn trig_arguments(expression:&str)->Vec<String>{
    let mut result=Vec::new();
    let bytes=expression.as_bytes();
    let mut cursor=0usize;
    while cursor<bytes.len(){
        let sin_at = expression[cursor..].find("sin(");
        let cos_at = expression[cursor..].find("cos(");
        let relative = match (sin_at, cos_at) {
            (Some(a), Some(b)) => Some(a.min(b)),
            (Some(a), None) => Some(a),
            (None, Some(b)) => Some(b),
            (None, None) => None,
        };
        let Some(relative) = relative else { break; };
        let start=cursor+relative;
        let open=start+expression[start..].find('(').unwrap();
        let mut depth=0i32;
        let mut quote=0u8;
        let mut escaped=false;
        let mut end=None;
        for i in open..bytes.len(){
            let ch=bytes[i];
            if quote!=0{
                if escaped{escaped=false;}
                else if ch==b'\\'{escaped=true;}
                else if ch==quote{quote=0;}
                continue;
            }
            if ch==b'"'||ch==b'\''{quote=ch;}
            else if ch==b'('{depth+=1;}
            else if ch==b')'{
                depth-=1;
                if depth==0{end=Some(i);break;}
            }
        }
        if let Some(end)=end{
            result.push(expression[open+1..end].to_owned());
            cursor=open+1;
        }else{break;}
    }
    result
}

/// Estimate linear angular coefficients for common affine trig inputs.
pub fn trig_coefficients(expression:&str,variable:&str)->Vec<f64>{
    let mut environment=HashMap::new();
    for name in ["limbSwing","limbSwingAmount","ageInTicks","netHeadYaw","headPitch","scaleFactor"]{
        environment.insert(name.to_owned(),0.0);
    }
    let mut coefficients=Vec::new();
    for argument in trig_arguments(expression){
        environment.insert(variable.to_owned(),0.0);
        let Some(zero)=evaluate(&argument,&environment).ok()else{continue;};
        environment.insert(variable.to_owned(),1.0);
        let Some(one)=evaluate(&argument,&environment).ok()else{continue;};
        let coefficient=(one-zero).abs();
        if coefficient>1e-8&&coefficient.is_finite(){coefficients.push(coefficient);}
    }
    coefficients
}

#[cfg(test)]
mod tests{
    use super::*;
    #[test]
    fn evaluates_java_animation_expressions(){
        let env=HashMap::from([
            ("limbSwing".to_owned(),std::f64::consts::FRAC_PI_2),
            ("limbSwingAmount".to_owned(),0.5),
            ("ageInTicks".to_owned(),10.0),
        ]);
        let value=evaluate("sin(limbSwing * 0.5F) * limbSwingAmount + cos(ageInTicks * 0.1)",&env).unwrap();
        assert!((value-0.8938556964614135).abs()<1e-8);
    }
    #[test]
    fn finds_trigonometric_source_coefficients(){
        let values=trig_coefficients("rotateX = cos(ageInTicks * 0.1F) + sin(limbSwing * 0.6662F)", "ageInTicks");
        assert!(values.iter().any(|value|(*value-0.1).abs()<1e-8));
    }
}
