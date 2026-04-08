use std::process::{Command, Child, Stdio};
use std::io::{Write, BufReader, BufRead};
use fasm_vm::{Value, fault::Fault};

pub struct SidecarPlugin {
    child: Child,
}

impl SidecarPlugin {
    pub fn new(cmd: &str, args: &[&str]) -> Self {
        let child = Command::new(cmd)
            .args(args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()
            .expect("Failed to spawn sidecar plugin process");

        Self { child }
    }

    pub fn call(&mut self, id: i32, arg: &Value) -> Result<Value, Fault> {
        let mut stdin = self.child.stdin.take().expect("Failed to open stdin");
        
        // Serialize input args to JSON tuple: [id, arg]
        let req_json = serde_json::to_string(&(id, arg))
            .unwrap_or_else(|_| "null".to_string());
            
        // Write out (add newline so sidecar can readline)
        writeln!(stdin, "{}", req_json).map_err(|_| Fault::BadSyscall)?;
        
        // Restore stdin to the child wrapper so it isn't dropped
        self.child.stdin = Some(stdin);
        
        // Read response blockingly
        let stdout = self.child.stdout.as_mut().expect("Failed to access stdout");
        let mut reader = BufReader::new(stdout);
        let mut line = String::new();
        
        reader.read_line(&mut line).map_err(|_| Fault::BadSyscall)?;
        
        // Attempt to parse JSON response back to Value structure
        if line.trim().is_empty() {
             return Ok(Value::Null); // Empty response defaults to null
        }
        
        match serde_json::from_str::<Value>(&line) {
            Ok(val) => Ok(val),
            Err(_) => Err(Fault::TypeMismatch) // Type mismatch if sidecar returned invalid FASM JSON
        }
    }
}

impl Drop for SidecarPlugin {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}
