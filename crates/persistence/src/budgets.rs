use super::*;

impl Store {
    pub fn set_budget(&mut self, id: &str, limit: i64) -> Result<()> {
        if !limeos_domain::identifier(id) || limit < 0 {
            return Err(Error(ErrorCode::InvalidInput));
        }
        self.write(|tx| { tx.execute("INSERT INTO budgets VALUES (?,?) ON CONFLICT(id) DO UPDATE SET limit_units=excluded.limit_units",params![id,limit]).map_err(durable)?; Ok(()) })
    }
    pub fn reserve(
        &mut self,
        request: &str,
        budget: &str,
        task: &str,
        price_revision: &str,
        amount: i64,
    ) -> Result<()> {
        if amount < 0
            || ![request, budget, task, price_revision]
                .iter()
                .all(|s| limeos_domain::identifier(s))
        {
            return Err(Error(ErrorCode::InvalidInput));
        }
        self.write(|tx| {
            let old:Option<(String,String,String,i64)>=tx.query_row("SELECT budget,task,price_revision,amount FROM reservations WHERE request=?",[request],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?))).optional().map_err(durable)?;
            if let Some(old)=old { return if old==(budget.into(),task.into(),price_revision.into(),amount) { Ok(()) } else { Err(Error(ErrorCode::Conflict)) }; }
            let limit:i64=tx.query_row("SELECT limit_units FROM budgets WHERE id=?",[budget],|r|r.get(0)).optional().map_err(durable)?.ok_or(Error(ErrorCode::NotFound))?;
            let used:i64=tx.query_row("SELECT coalesce(sum(coalesce(settled,amount)),0) FROM reservations WHERE budget=?",[budget],|r|r.get(0)).map_err(durable)?;
            if amount>limit.saturating_sub(used) { return Err(Error(ErrorCode::Forbidden)); }
            tx.execute("INSERT INTO reservations VALUES (?,?,?,?,?,NULL)",params![request,budget,task,price_revision,amount]).map_err(durable)?; Ok(())
        })
    }
    pub fn settle(&mut self, request: &str, actual: i64) -> Result<()> {
        self.write(|tx| {
            let (amount, settled): (i64, Option<i64>) = tx
                .query_row(
                    "SELECT amount,settled FROM reservations WHERE request=?",
                    [request],
                    |r| Ok((r.get(0)?, r.get(1)?)),
                )
                .optional()
                .map_err(durable)?
                .ok_or(Error(ErrorCode::NotFound))?;
            if actual < 0 || actual > amount || settled.is_some_and(|a| a != actual) {
                return Err(Error(ErrorCode::Conflict));
            }
            tx.execute(
                "UPDATE reservations SET settled=? WHERE request=? AND settled IS NULL",
                params![actual, request],
            )
            .map_err(durable)?;
            Ok(())
        })
    }
}
