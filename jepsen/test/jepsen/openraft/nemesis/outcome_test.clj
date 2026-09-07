(ns jepsen.openraft.nemesis.outcome-test
  (:require [clojure.test :refer [deftest is]]
            [jepsen.openraft.nemesis.outcome :as outcome]))

(deftest categorizes-target-scale
  (is (= [:one :minority :majority :majority :all]
         (mapv (partial outcome/target-category 5)
               (range 1 6))))
  (is (= :one (outcome/target-category 1 1))))
