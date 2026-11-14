class Parser {
  void close() {}
  void work() {}
  RuntimeException handler(Throwable ex) { return new IllegalStateException(ex); }
  void stopParsing(Throwable error) { if (error != null) { try {close();} catch(Throwable failure) {} if (error instanceof RuntimeException) throw (RuntimeException)error; else if(error instanceof Error) throw (Error)error; else throw new IllegalStateException(error); } else { close(); } }
  void parse() { try { work(); } catch(Throwable ex) { try { ex = handler(ex); } finally { stopParsing(ex); } } }
  void parseNext() { try { work(); } catch(Throwable ex) { try { ex = handler(ex); } finally { stopParsing(ex); } } }
  void ordinary() { try { work(); } catch(Exception ex) { System.err.println(ex); } }
}
