(ns jepsen.openraft.db-test
  (:require [clojure.test :refer [deftest is testing]]
            [jepsen.control :as c]
            [jepsen.db :as db]
            [jepsen.openraft.db :as openraft-db]))

(def test-config
  {:api-port 21001
   :raft-port 22001
   :snapshot-threshold 250})

(defn- command-kind [command]
  (cond
    (= :bash (first command)) :probe
    (= :killall (first command)) (case (nth command 2)
                                   9 :kill
                                   "STOP" :pause
                                   "CONT" :resume)
    (= :rm (first command)) :remove
    (= :mkdir (first command)) :mkdir
    (some #{:start-stop-daemon} command) :start))

(defn- argument-after [command argument]
  (let [index (.indexOf command argument)]
    (when-not (neg? index)
      (nth command (inc index)))))

(deftest starts-and-confirms-the-test-app
  (let [calls (atom [])
        running? (atom false)
        database (openraft-db/db {})]
    (with-redefs [c/exec
                  (fn [& command]
                    (swap! calls conj command)
                    (case (command-kind command)
                      :probe (if @running? "running" "absent")
                      :start (do (reset! running? true) "")
                      ""))]
      (is (= :start-confirmed (db/start! database test-config "n1"))))
    (let [start-command (first (filter #(= :start (command-kind %)) @calls))]
      (is (some #{:--oknodo} start-command))
      (is (= "n1" (argument-after start-command :--id)))
      (is (= "n1:21001" (argument-after start-command :--api-addr)))
      (is (= "n1:22001" (argument-after start-command :--raft-addr)))
      (is (= 250 (argument-after start-command :--snapshot-threshold))))))

(deftest uses-explicit-process-evidence-when-starting
  (let [database (openraft-db/db {})]
    (testing "an already running process is not started again"
      (let [starts (atom 0)]
        (with-redefs [c/exec
                      (fn [& command]
                        (case (command-kind command)
                          :probe "running"
                          :start (do (swap! starts inc) "")
                          ""))]
          (is (= :already-running
                 (db/start! database test-config "n1"))))
        (is (zero? @starts))))

    (testing "a paused process is not treated as already running"
      (let [starts (atom 0)
            error (with-redefs [c/exec
                                (fn [& command]
                                  (case (command-kind command)
                                    :probe "paused"
                                    :start (do (swap! starts inc) "")
                                    ""))]
                    (try
                      (db/start! database test-config "n1")
                      nil
                      (catch Exception e
                        e)))]
        (is (= :unexpected-existing-process-state
               (:kind (ex-data error))))
        (is (= :paused (:state (ex-data error))))
        (is (zero? @starts))))

    (testing "an exit-one start failure propagates unchanged"
      (let [error (ex-info "permission denied"
                           {:type :jepsen.control/nonzero-exit
                            :exit 1})
            thrown (with-redefs [c/exec
                                 (fn [& command]
                                   (case (command-kind command)
                                     :probe "absent"
                                     :start (throw error)
                                     ""))]
                     (try
                       (db/start! database test-config "n1")
                       nil
                       (catch Exception e
                         e)))]
        (is (identical? error thrown))))

    (testing "a background start that never appears times out"
      (let [error
            (with-redefs-fn
              {#'c/exec
               (fn [& command]
                 (case (command-kind command)
                   :probe "absent"
                   ""))
               #'openraft-db/process-confirm-timeout-ms 0}
              #(try
                 (db/start! database test-config "n1")
                 nil
                 (catch Exception e
                   e)))]
        (is (= :process-confirmation-timeout
               (:kind (ex-data error))))))))

(deftest pauses-and-resumes-processes-with-strict-evidence
  (let [database (openraft-db/db {})]
    (testing "a pause signal is confirmed"
      (let [state (atom :running)
            calls (atom [])]
        (with-redefs [c/exec
                      (fn [& command]
                        (swap! calls conj command)
                        (case (command-kind command)
                          :probe (name @state)
                          :pause (do (reset! state :paused) "")
                          ""))]
          (is (= :paused (db/pause! database {} "n1"))))
        (is (= [:probe :pause :probe]
               (mapv command-kind @calls)))))

    (testing "a resume signal is confirmed"
      (let [state (atom :paused)
            calls (atom [])]
        (with-redefs [c/exec
                      (fn [& command]
                        (swap! calls conj command)
                        (case (command-kind command)
                          :probe (name @state)
                          :resume (do (reset! state :running) "")
                          ""))]
          (is (= :resumed (db/resume! database {} "n1"))))
        (is (= [:probe :resume :probe]
               (mapv command-kind @calls)))))

    (doseq [[label operation]
            [[:pause #(db/pause! database {} "n1")]
             [:resume #(db/resume! database {} "n1")]]]
      (testing (str (name label) " records a pre-probe absence")
        (let [calls (atom [])]
          (with-redefs [c/exec
                        (fn [& command]
                          (swap! calls conj command)
                          (case (command-kind command)
                            :probe "absent"
                            ""))]
            (is (= :target-absent (operation))))
          (is (= [:probe] (mapv command-kind @calls))))))

    (doseq [[label process-state operation]
            [[:pause :running #(db/pause! database {} "n1")]
             [:resume :paused #(db/resume! database {} "n1")]]]
      (testing (str (name label) " records an explicit exit race")
        (let [race (ex-info "no process found"
                            {:type :jepsen.control/nonzero-exit
                             :exit 1
                             :err (str openraft-db/process-name
                                       ": no process found")})]
          (with-redefs [c/exec
                        (fn [& command]
                          (case (command-kind command)
                            :probe (name process-state)
                            (:pause :resume) (throw race)
                            ""))]
            (is (= :target-already-exited (operation)))))))

    (doseq [[label process-state operation]
            [[:pause :running #(db/pause! database {} "n1")]
             [:resume :paused #(db/resume! database {} "n1")]]]
      (testing (str (name label) " propagates control failures")
        (let [error (ex-info "permission denied"
                             {:type :jepsen.control/nonzero-exit
                              :exit 1
                              :err "permission denied"})
              thrown (with-redefs [c/exec
                                   (fn [& command]
                                     (case (command-kind command)
                                       :probe (name process-state)
                                       (:pause :resume) (throw error)
                                       ""))]
                       (try
                         (operation)
                         nil
                         (catch Exception e
                           e)))]
          (is (identical? error thrown)))))

    (doseq [[label process-state operation]
            [[:pause :running #(db/pause! database {} "n1")]
             [:resume :paused #(db/resume! database {} "n1")]]]
      (testing (str (name label) " has bounded confirmation")
        (let [error
              (with-redefs-fn
                {#'c/exec
                 (fn [& command]
                   (case (command-kind command)
                     :probe (name process-state)
                     ""))
                 #'openraft-db/process-confirm-timeout-ms 0}
                #(try
                   (operation)
                   nil
                   (catch Exception e
                     e)))]
          (is (= :process-confirmation-timeout
                 (:kind (ex-data error)))))))))

(deftest stops-processes-with-strict-evidence
  (let [database (openraft-db/db {})]
    (testing "a pre-probe absence is a structured no-op"
      (let [calls (atom [])]
        (with-redefs [c/exec
                      (fn [& command]
                        (swap! calls conj command)
                        (case (command-kind command)
                          :probe "absent"
                          ""))]
          (is (= :target-absent (db/kill! database {} "n1"))))
        (is (= [:probe :remove]
               (mapv command-kind @calls)))))

    (testing "a successful kill is confirmed absent"
      (let [running? (atom true)
            calls (atom [])]
        (with-redefs [c/exec
                      (fn [& command]
                        (swap! calls conj command)
                        (case (command-kind command)
                          :probe (if @running? "running" "absent")
                          :kill (do (reset! running? false) "")
                          ""))]
          (is (= :killed (db/kill! database {} "n1"))))
        (is (= [:probe :kill :probe :remove]
               (mapv command-kind @calls)))))

    (testing "an explicit exit race is skipped without polling"
      (let [calls (atom [])
            race (ex-info "no process found"
                          {:type :jepsen.control/nonzero-exit
                           :exit 1
                           :err "openraft-jepsen-app: no process found"})]
        (with-redefs [c/exec
                      (fn [& command]
                        (swap! calls conj command)
                        (case (command-kind command)
                          :probe "running"
                          :kill (throw race)
                          ""))]
          (is (= :target-already-exited
                 (db/kill! database {} "n1"))))
        (is (= [:probe :kill :remove]
               (mapv command-kind @calls)))))

    (testing "permission failures are never treated as absence"
      (let [removed? (atom false)
            error (ex-info "permission denied"
                           {:type :jepsen.control/nonzero-exit
                            :exit 1
                            :err "permission denied"})
            thrown (with-redefs [c/exec
                                 (fn [& command]
                                   (case (command-kind command)
                                     :probe "running"
                                     :kill (throw error)
                                     :remove (do (reset! removed? true) "")))]
                     (try
                       (db/kill! database {} "n1")
                       nil
                       (catch Exception e
                         e)))]
        (is (identical? error thrown))
        (is (false? @removed?))))

    (testing "mixed control diagnostics are not an explicit exit race"
      (let [error (ex-info "kill failed"
                           {:type :jepsen.control/nonzero-exit
                            :exit 1
                            :err (str "permission denied\n"
                                      openraft-db/process-name
                                      ": no process found")})
            thrown (with-redefs [c/exec
                                 (fn [& command]
                                   (case (command-kind command)
                                     :probe "running"
                                     :kill (throw error)
                                     ""))]
                     (try
                       (db/kill! database {} "n1")
                       nil
                       (catch Exception e
                         e)))]
        (is (identical? error thrown))))

    (testing "a process that remains running fails bounded confirmation"
      (let [error
            (with-redefs-fn
              {#'c/exec
               (fn [& command]
                 (case (command-kind command)
                   :probe "running"
                   ""))
               #'openraft-db/process-confirm-timeout-ms 0}
              #(try
                 (db/kill! database {} "n1")
                 nil
                 (catch Exception e
                   e)))]
        (is (= :process-confirmation-timeout
               (:kind (ex-data error))))))))

(deftest process-control-preserves-interruptions
  (doseq [[label make-error]
          [[:interrupted #(InterruptedException. "interrupted")]
           [:interrupted-io
            #(java.io.InterruptedIOException. "interrupted")]
           [:closed-by-interrupt
            #(java.nio.channels.ClosedByInterruptException.)]
           [:wrapped
            #(ex-info "interrupted"
                      {:kind :interrupted
                       :type :jepsen.control/nonzero-exit
                       :exit 1
                       :err "no process found"})]]]
    (testing (name label)
      (Thread/interrupted)
      (try
        (let [error (make-error)
              database (openraft-db/db {})
              [thrown interrupted?]
              (with-redefs [c/exec (fn [& _] (throw error))]
                (try
                  (db/kill! database {} "n1")
                  [nil (.isInterrupted (Thread/currentThread))]
                  (catch Exception e
                    [e (.isInterrupted (Thread/currentThread))])))]
          (is (identical? error thrown))
          (is interrupted?))
        (finally
          (Thread/interrupted))))))

(deftest every-process-control-stage-preserves-interruptions
  (let [database (openraft-db/db {})
        stages
        [{:label :remove-pid
          :operation #(db/kill! database {} "n1")
          :exec (fn [error command]
                  (case (command-kind command)
                    :probe "absent"
                    :remove (throw error)
                    ""))}
         {:label :kill-signal
          :operation #(db/kill! database {} "n1")
          :exec (fn [error command]
                  (case (command-kind command)
                    :probe "running"
                    :kill (throw error)
                    ""))}
         {:label :pause-signal
          :operation #(db/pause! database {} "n1")
          :exec (fn [error command]
                  (case (command-kind command)
                    :probe "running"
                    :pause (throw error)
                    ""))}
         {:label :resume-signal
          :operation #(db/resume! database {} "n1")
          :exec (fn [error command]
                  (case (command-kind command)
                    :probe "paused"
                    :resume (throw error)
                    ""))}
         {:label :start-command
          :operation #(db/start! database test-config "n1")
          :exec (fn [error command]
                  (case (command-kind command)
                    :probe "absent"
                    :start (throw error)
                    ""))}]]
    (doseq [{:keys [label operation exec]} stages]
      (testing (name label)
        (Thread/interrupted)
        (try
          (let [error (InterruptedException. "interrupted")
                [thrown interrupted?]
                (with-redefs [c/exec (fn [& command]
                                       (exec error command))]
                  (try
                    (operation)
                    [nil (.isInterrupted (Thread/currentThread))]
                    (catch Exception e
                      [e (.isInterrupted (Thread/currentThread))])))]
            (is (identical? error thrown))
            (is interrupted?))
          (finally
            (Thread/interrupted)))))))

(deftest passes-the-process-pattern-as-a-separate-argument
  (let [calls (atom [])
        adversarial "/tmp/openraft app; touch /tmp/not-run"
        database (openraft-db/db {})]
    (with-redefs [openraft-db/binary adversarial
                  c/exec (fn [& command]
                           (swap! calls conj command)
                           (if (= :probe (command-kind command))
                             "absent"
                             ""))]
      (is (= :target-absent (db/kill! database {} "n1"))))
    (let [probe (first @calls)]
      (is (not (.contains ^String (nth probe 2) adversarial)))
      (is (= (str "^" adversarial "([[:space:]].*)?$")
             (last probe))))))
